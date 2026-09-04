use ketchup_core::bottle_m6::ExactRevolveRequest;
use ketchup_core::document::{
    BOTTLE_SHELL_OPENING_FACE_ROLE, BOTTLE_SHOULDER_EDGE_ROLE, BooleanOperation,
    BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand, CanonicalError, CommandBatch,
    DefinitionId, DerivedIdentity, Dimension, DimensionDisplayUnit, DimensionPresentation,
    DimensionReferenceHealth, DocumentId, DocumentStore, EvaluationIdentity, FeatureId,
    FeatureKind, FeatureParameterBinding, FeatureParameterFreshness, FeatureParameterStaleReason,
    FeatureParameterTarget, LoftSection, MeshAuthority, NodeId, OccurrenceId, ParameterPath,
    ParameterValueType, PersistentDimension, PersistentDimensionId, PersistentDimensionTarget,
    PortSpec, ProfileSegment, RuleOutput, SlotPath, SlotSegment, StableEdgeRole, StableFaceRole,
    Transform,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V7, ExactBRepGraph, ExactBRepPlanarSegment,
    MAX_EXACT_BREP_LOFT_CONTROL_POINTS, MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM,
    MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM,
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
    PlanarOffsetWorkerEvidence, build_planar_offset_package, canonical_reference_lineage_digest,
    line_arc_d_arc_only_side_overlap,
};
use ketchup_core::sketch::{
    PrincipalPlane, SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
};
use ketchup_exact::{
    CutMode, CylinderToolSpec, ExactBackend, GeometryErrorCode, PlanarProfileSegment,
    RectangleExtrudeSpec, ReferenceResolution, StabilityClass, capture_circle_extrusion_references,
    capture_circular_pocket_references, capture_circular_split_references,
    capture_circular_through_cut_references, capture_guaranteed_references,
    capture_polygon_through_cut_references, resolve_subshape_reference,
};
use ketchup_interaction::exact_projection::ExactInteractionProjection;
use ketchup_interaction::{Ray, Vec3};
use ketchup_scheduler::{ExactWorkerSupervisor, M3EvaluationError, WorkerError};
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

fn generated_rectangle_dimension_samples() -> Vec<[f64; 3]> {
    let mut samples = vec![[0.5, 0.75, 0.25], [1.0, 1.0, 1.0], [250.0, 180.0, 120.0]];
    let mut state = 0x4b45_5453_5550_2026_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 11) as f64) / ((1_u64 << 53) as f64)
    };
    samples.extend((0..9).map(|_| {
        [
            0.5 + next() * 249.5,
            0.5 + next() * 179.5,
            0.25 + next() * 119.75,
        ]
    }));
    samples
}

#[test]
fn generated_rectangle_samples_are_reproducible_bounded_and_round_trip_metamorphic() {
    let samples = generated_rectangle_dimension_samples();
    assert_eq!(samples, generated_rectangle_dimension_samples());
    assert_eq!(samples.len(), 12);
    assert_eq!(samples[0], [0.5, 0.75, 0.25]);
    assert_eq!(samples[2], [250.0, 180.0, 120.0]);

    for (sample_index, [base_width, base_depth, base_height]) in samples.iter().copied().enumerate()
    {
        assert!((0.5..=250.0).contains(&base_width));
        assert!((0.5..=180.0).contains(&base_depth));
        assert!((0.25..=120.0).contains(&base_height));
        assert!(samples.iter().skip(sample_index + 1).all(|candidate| {
            candidate[0].to_bits() != base_width.to_bits()
                || candidate[1].to_bits() != base_depth.to_bits()
                || candidate[2].to_bits() != base_height.to_bits()
        }));

        let mut volumes = [0.0; 3];
        for (variant, [width, depth, height]) in [
            [base_width, base_depth, base_height],
            [base_depth, base_width, base_height],
            [base_width * 1.5, base_depth * 1.5, base_height * 1.5],
        ]
        .into_iter()
        .enumerate()
        {
            let case_id = format!("generated-verifier-{sample_index}-{variant}");
            assert!(width.is_finite() && width > 0.0, "{case_id}");
            assert!(depth.is_finite() && depth > 0.0, "{case_id}");
            assert!(height.is_finite() && height > 0.0, "{case_id}");

            let document = rectangle_document(width, depth, height);
            let snapshot = document.current();
            let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
            let reopened =
                match ketchup_core::persistence::load(&ketchup_core::persistence::save(&snapshot))
                    .unwrap()
                {
                    ketchup_core::persistence::LoadOutcome::Editable { document, .. } => document,
                    ketchup_core::persistence::LoadOutcome::ReviewOnly(_) => {
                        panic!("{case_id}: generated rectangle must reopen editable")
                    }
                };
            let reopened_snapshot = reopened.current();
            assert_eq!(
                reopened_snapshot.canonical_digest(),
                snapshot.canonical_digest()
            );
            assert_eq!(
                ExactFeatureChainRequest::from_snapshot(&reopened_snapshot, DEFINITION).unwrap(),
                request,
                "{case_id}: exact request changed across save/open"
            );

            let first = ExactBackend::new()
                .extrude_rectangle(RectangleExtrudeSpec {
                    width_mm: width,
                    depth_mm: depth,
                    height_mm: height,
                })
                .unwrap();
            let repeated = ExactBackend::new()
                .extrude_rectangle(RectangleExtrudeSpec {
                    width_mm: width,
                    depth_mm: depth,
                    height_mm: height,
                })
                .unwrap();
            assert_eq!(
                first.body.result_fingerprint, repeated.body.result_fingerprint,
                "{case_id}: direct exact evaluation is not deterministic"
            );
            volumes[variant] = first.body.topology.volume_mm3;
        }

        assert!(
            (volumes[1] - volumes[0]).abs() <= volumes[0].max(1.0) * 1.0e-10,
            "sample {sample_index}: axis swap changed exact volume"
        );
        assert!(
            (volumes[2] - volumes[0] * 1.5_f64.powi(3)).abs() <= volumes[2].max(1.0) * 1.0e-10,
            "sample {sample_index}: exact volume did not scale cubically"
        );
    }
}

#[test]
fn generated_rectangle_properties_have_stable_identities_and_survive_save_open() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let samples = generated_rectangle_dimension_samples();
    let mut observed_cases = 0;
    let mut observed_roles = 0;
    let mut observed_volumes = BTreeMap::<(usize, u8), f64>::new();

    for (sample_index, [base_width, base_depth, base_height]) in samples.iter().copied().enumerate()
    {
        for (variant, [width, depth, height]) in [
            [base_width, base_depth, base_height],
            [base_depth, base_width, base_height],
            [base_width * 1.5, base_depth * 1.5, base_height * 1.5],
        ]
        .into_iter()
        .enumerate()
        {
            let case_id = format!("generated-{sample_index}-{variant}");
            let mut document = rectangle_document(width, depth, height);
            let snapshot = document.current();
            let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
            let package = supervisor.evaluate_rectangle(&request).unwrap();
            let repeated = supervisor.evaluate_rectangle(&request).unwrap();
            assert!(package.is_current(&snapshot));
            assert_eq!(package.identity, repeated.identity, "{case_id}");
            assert_eq!(package.references, repeated.references, "{case_id}");
            assert_eq!(package.vertices, repeated.vertices, "{case_id}");
            assert_eq!(package.triangles, repeated.triangles, "{case_id}");
            assert_eq!(package.vertices.len(), 8);
            assert_eq!(package.triangles.len(), 12);

            let direct = ExactBackend::new()
                .extrude_rectangle(RectangleExtrudeSpec {
                    width_mm: width,
                    depth_mm: depth,
                    height_mm: height,
                })
                .unwrap();
            let expected_volume = width * depth * height;
            assert!(
                (direct.body.topology.volume_mm3 - expected_volume).abs()
                    <= expected_volume.max(1.0) * 1.0e-10,
                "{case_id}: exact volume {} differs from {expected_volume}",
                direct.body.topology.volume_mm3
            );
            assert_eq!(
                [
                    direct.body.topology.vertex_count,
                    direct.body.topology.edge_count,
                    direct.body.topology.face_count,
                    direct.body.topology.shell_count,
                    direct.body.topology.solid_count,
                ],
                [8, 12, 6, 1, 1],
                "{case_id}: rectangular extrusion topology changed"
            );
            observed_volumes.insert(
                (sample_index, variant as u8),
                direct.body.topology.volume_mm3,
            );
            assert_eq!(
                package.identity.result_fingerprint, direct.body.result_fingerprint,
                "{case_id}: worker and direct exact evaluation diverged"
            );
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
            reopened_references
                .sort_by(|left, right| left.lineage_digest.cmp(&right.lineage_digest));
            expected_references
                .sort_by(|left, right| left.lineage_digest.cmp(&right.lineage_digest));
            assert_eq!(reopened_references, expected_references, "{case_id}");
            observed_cases += 1;
        }
    }

    assert_eq!(observed_cases, samples.len() * 3);
    assert_eq!(observed_roles, samples.len() * 9);
    for sample_index in 0..samples.len() {
        let base = observed_volumes[&(sample_index, 0)];
        let axis_swapped = observed_volumes[&(sample_index, 1)];
        let uniformly_scaled = observed_volumes[&(sample_index, 2)];
        assert!(
            (axis_swapped - base).abs() <= base.max(1.0) * 1.0e-10,
            "sample {sample_index}: swapping profile axes changed volume"
        );
        assert!(
            (uniformly_scaled - base * 1.5_f64.powi(3)).abs()
                <= uniformly_scaled.max(1.0) * 1.0e-10,
            "sample {sample_index}: uniform scale did not scale volume cubically"
        );
    }
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
                        path: ParameterPath::new("height").unwrap(),
                        value_type: ParameterValueType::Length,
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
    let target =
        FeatureParameterTarget::new(EXTRUSION, "height", ParameterValueType::Length).unwrap();
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
                ExactFaceRole::Top
                | ExactFaceRole::Bottom
                | ExactFaceRole::East
                | ExactFaceRole::West => PROFILE,
                ExactFaceRole::CutLinear
                | ExactFaceRole::CutArc
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
fn scheduler_evaluates_arc_only_side_overlapping_d_profile_through_cut() {
    let cases = [
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 55.0],
                end_mm: [40.0, 55.0],
                center_mm: [30.0, 55.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 55.0],
                end_mm: [20.0, 55.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 5.0],
                end_mm: [40.0, 5.0],
                center_mm: [30.0, 5.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 5.0],
                end_mm: [20.0, 5.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [95.0, 20.0],
                end_mm: [95.0, 40.0],
                center_mm: [95.0, 30.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [95.0, 40.0],
                end_mm: [95.0, 20.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [5.0, 20.0],
                end_mm: [5.0, 40.0],
                center_mm: [5.0, 30.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [5.0, 40.0],
                end_mm: [5.0, 20.0],
            },
        ],
    ];
    let expected_overlap_area = 50.0 * std::f64::consts::PI / 3.0 + 25.0 * 3.0_f64.sqrt();
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();

    for (index, segments) in cases.into_iter().enumerate() {
        let base = ExactBackend::new()
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: 100.0,
                depth_mm: 60.0,
                height_mm: 18.0,
            })
            .unwrap();
        let direct = ExactBackend::new()
            .cut_mixed_profile(&base.body, &planar_segments(segments.clone()), -1.0, 20.0)
            .unwrap();
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [16, 24, 10, 1, 1]
        );
        let mut document =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, overlap_bounds) = profile
            .d_profile_arc_only_clipped_side_overlap(100.0, 60.0)
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert!(overlap_bounds.into_iter().all(f64::is_finite));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        assert_eq!(
            package
                .reference(ExactFaceRole::CutLinear)
                .unwrap()
                .profile_feature_id,
            CUT_PROFILE
        );
        assert_closed_manifold(&package);

        let directory = tempfile::tempdir().unwrap();
        let step_path = directory
            .path()
            .join(format!("d-arc-side-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
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
        assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(document.current().revision_id(), before_revision);
        assert_eq!(document.current().canonical_digest(), before_digest);

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
            .join(format!("stale-d-arc-side-cut-{index}.step"));
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

    let invalid_cases = [
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 50.0],
                end_mm: [40.0, 50.0],
                center_mm: [30.0, 50.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 50.0],
                end_mm: [20.0, 50.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 65.0],
                end_mm: [40.0, 65.0],
                center_mm: [30.0, 65.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 65.0],
                end_mm: [20.0, 65.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [1.0, 51.0],
                end_mm: [9.0, 59.0],
                center_mm: [5.0, 55.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [9.0, 59.0],
                end_mm: [1.0, 51.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 55.0],
                end_mm: [40.0, 55.0],
                center_mm: [30.0, 54.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 55.0],
                end_mm: [20.0, 55.0],
            },
        ],
    ];
    for segments in invalid_cases {
        assert_eq!(
            line_arc_d_arc_only_side_overlap(&segments, true, 100.0, 60.0),
            None
        );
    }
}

#[test]
fn scheduler_evaluates_arc_only_side_overlapping_d_profile_split() {
    let cases = [
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 55.0],
                    end_mm: [40.0, 55.0],
                    center_mm: [30.0, 55.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 55.0],
                    end_mm: [20.0, 55.0],
                },
            ],
            [20.0, 55.0, 40.0, 60.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 5.0],
                    end_mm: [40.0, 5.0],
                    center_mm: [30.0, 5.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 5.0],
                    end_mm: [20.0, 5.0],
                },
            ],
            [20.0, 0.0, 40.0, 5.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [95.0, 20.0],
                    end_mm: [95.0, 40.0],
                    center_mm: [95.0, 30.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [95.0, 40.0],
                    end_mm: [95.0, 20.0],
                },
            ],
            [95.0, 20.0, 100.0, 40.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [5.0, 20.0],
                    end_mm: [5.0, 40.0],
                    center_mm: [5.0, 30.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [5.0, 40.0],
                    end_mm: [5.0, 20.0],
                },
            ],
            [0.0, 20.0, 5.0, 40.0],
        ),
    ];
    let expected_overlap_area = 50.0 * std::f64::consts::PI / 3.0 + 25.0 * 3.0_f64.sqrt();
    let expected_volume = 108_000.0;
    let expected_direct_topology = [16, 26, 13, 2, 2];
    let expected_step_topology = [24, 36, 16, 2, 2];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let split = backend
            .split_mixed_profile(&base.body, &planar_segments(segments.clone()), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                split.body.topology.vertex_count,
                split.body.topology.edge_count,
                split.body.topology.face_count,
                split.body.topology.shell_count,
                split.body.topology.solid_count,
            ],
            expected_direct_topology
        );
        assert!((split.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let mut document =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Split, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.d_profile_arc_only_clipped_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        for (actual, expected) in overlap_bounds.into_iter().zip(expected_overlap_bounds) {
            assert!((actual - expected).abs() < 1.0e-9);
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let arc_side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(arc_side.profile_feature_id, CUT_PROFILE);
        assert!(arc_side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 2);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let directory = tempfile::tempdir().unwrap();
        let step_path = directory
            .path()
            .join(format!("d-arc-side-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-d-arc-side-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology
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
    }

    for segments in [
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 50.0],
                end_mm: [40.0, 50.0],
                center_mm: [30.0, 50.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 50.0],
                end_mm: [20.0, 50.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 65.0],
                end_mm: [40.0, 65.0],
                center_mm: [30.0, 65.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 65.0],
                end_mm: [20.0, 65.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [1.0, 51.0],
                end_mm: [9.0, 59.0],
                center_mm: [5.0, 55.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [9.0, 59.0],
                end_mm: [1.0, 51.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 55.0],
                end_mm: [40.0, 55.0],
                center_mm: [30.0, 54.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 55.0],
                end_mm: [20.0, 55.0],
            },
        ],
    ] {
        let rejected =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Split, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_only_side_overlapping_d_profile_intersection() {
    let cases = [
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 55.0],
                    end_mm: [40.0, 55.0],
                    center_mm: [30.0, 55.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 55.0],
                    end_mm: [20.0, 55.0],
                },
            ],
            [20.0, 55.0, 40.0, 60.0],
            [30.0, 55.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 5.0],
                    end_mm: [40.0, 5.0],
                    center_mm: [30.0, 5.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 5.0],
                    end_mm: [20.0, 5.0],
                },
            ],
            [20.0, 0.0, 40.0, 5.0],
            [30.0, 5.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [95.0, 20.0],
                    end_mm: [95.0, 40.0],
                    center_mm: [95.0, 30.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [95.0, 40.0],
                    end_mm: [95.0, 20.0],
                },
            ],
            [95.0, 20.0, 100.0, 40.0],
            [95.0, 30.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [5.0, 20.0],
                    end_mm: [5.0, 40.0],
                    center_mm: [5.0, 30.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [5.0, 40.0],
                    end_mm: [5.0, 20.0],
                },
            ],
            [0.0, 20.0, 5.0, 40.0],
            [5.0, 30.0],
        ),
    ];
    let expected_overlap_area = 50.0 * std::f64::consts::PI / 3.0 + 25.0 * 3.0_f64.sqrt();
    let expected_volume = expected_overlap_area * 18.0;
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();

    for (index, (segments, expected_bounds, center)) in cases.into_iter().enumerate() {
        let direct = backend
            .common_mixed_profile(&base.body, &planar_segments(segments.clone()), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [8, 12, 6, 1, 1]
        );
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let mut document =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        for (actual, expected) in request.expected_bounds_mm().into_iter().flatten().zip([
            expected_bounds[0],
            expected_bounds[1],
            0.0,
            expected_bounds[2],
            expected_bounds[3],
            18.0,
        ]) {
            assert!((actual - expected).abs() < 1.0e-9);
        }
        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let arc_side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(arc_side.profile_feature_id, CUT_PROFILE);
        assert!(arc_side.has_valid_lineage());
        let arc_triangles = package
            .triangles
            .iter()
            .filter(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
            .collect::<Vec<_>>();
        assert!(!arc_triangles.is_empty());
        for triangle in arc_triangles {
            for vertex_index in triangle.vertex_indices {
                let [x, y, _] = package.vertices[vertex_index as usize].position_mm;
                assert!(((x - center[0]).hypot(y - center[1]) - 10.0).abs() < 1.0e-6);
            }
        }
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let directory = tempfile::tempdir().unwrap();
        let step_path = directory
            .path()
            .join(format!("d-arc-side-intersect-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-d-arc-side-intersect-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
    }

    let invalid_cases = [
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 50.0],
                end_mm: [40.0, 50.0],
                center_mm: [30.0, 50.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 50.0],
                end_mm: [20.0, 50.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 65.0],
                end_mm: [40.0, 65.0],
                center_mm: [30.0, 65.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 65.0],
                end_mm: [20.0, 65.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [1.0, 51.0],
                end_mm: [9.0, 59.0],
                center_mm: [5.0, 55.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [9.0, 59.0],
                end_mm: [1.0, 51.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 55.0],
                end_mm: [40.0, 55.0],
                center_mm: [30.0, 54.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 55.0],
                end_mm: [20.0, 55.0],
            },
        ],
    ];
    for segments in invalid_cases {
        let rejected =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_only_side_overlapping_d_profile_union() {
    let cases = [
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 55.0],
                    end_mm: [40.0, 55.0],
                    center_mm: [30.0, 55.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 55.0],
                    end_mm: [20.0, 55.0],
                },
            ],
            [20.0, 55.0, 40.0, 60.0],
            [[0.0, 0.0, 0.0], [100.0, 65.0, 18.0]],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 5.0],
                    end_mm: [40.0, 5.0],
                    center_mm: [30.0, 5.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 5.0],
                    end_mm: [20.0, 5.0],
                },
            ],
            [20.0, 0.0, 40.0, 5.0],
            [[0.0, -5.0, 0.0], [100.0, 60.0, 18.0]],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [95.0, 20.0],
                    end_mm: [95.0, 40.0],
                    center_mm: [95.0, 30.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [95.0, 40.0],
                    end_mm: [95.0, 20.0],
                },
            ],
            [95.0, 20.0, 100.0, 40.0],
            [[0.0, 0.0, 0.0], [105.0, 60.0, 18.0]],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [5.0, 20.0],
                    end_mm: [5.0, 40.0],
                    center_mm: [5.0, 30.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [5.0, 40.0],
                    end_mm: [5.0, 20.0],
                },
            ],
            [0.0, 20.0, 5.0, 40.0],
            [[-5.0, 0.0, 0.0], [100.0, 60.0, 18.0]],
        ),
    ];
    let expected_overlap_area = 50.0 * std::f64::consts::PI / 3.0 + 25.0 * 3.0_f64.sqrt();
    let profile_area = 50.0 * std::f64::consts::PI;
    let expected_volume = (6_000.0 + profile_area - expected_overlap_area) * 18.0;
    let expected_topology = [12, 18, 8, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();

    for (index, (segments, expected_overlap_bounds, expected_bounds)) in
        cases.into_iter().enumerate()
    {
        let union = backend
            .fuse_mixed_profile(&base.body, &planar_segments(segments.clone()), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                union.body.topology.vertex_count,
                union.body.topology.edge_count,
                union.body.topology.face_count,
                union.body.topology.shell_count,
                union.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let mut document =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.d_profile_arc_only_clipped_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        for (actual, expected) in overlap_bounds.into_iter().zip(expected_overlap_bounds) {
            assert!((actual - expected).abs() < 1.0e-9);
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            union.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, expected_bounds);
        let arc_side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(arc_side.profile_feature_id, CUT_PROFILE);
        assert!(arc_side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let directory = tempfile::tempdir().unwrap();
        let step_path = directory
            .path()
            .join(format!("d-arc-side-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-d-arc-side-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
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
    }

    for segments in [
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 50.0],
                end_mm: [40.0, 50.0],
                center_mm: [30.0, 50.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 50.0],
                end_mm: [20.0, 50.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 55.0],
                end_mm: [40.0, 55.0],
                center_mm: [30.0, 54.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 55.0],
                end_mm: [20.0, 55.0],
            },
        ],
    ] {
        let rejected =
            line_arc_d_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_only_side_overlapping_d_profile_pocket() {
    let cases = [
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 55.0],
                    end_mm: [40.0, 55.0],
                    center_mm: [30.0, 55.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 55.0],
                    end_mm: [20.0, 55.0],
                },
            ],
            [20.0, 55.0, 40.0, 60.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [20.0, 5.0],
                    end_mm: [40.0, 5.0],
                    center_mm: [30.0, 5.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [40.0, 5.0],
                    end_mm: [20.0, 5.0],
                },
            ],
            [20.0, 0.0, 40.0, 5.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [95.0, 20.0],
                    end_mm: [95.0, 40.0],
                    center_mm: [95.0, 30.0],
                    clockwise: false,
                },
                ProfileSegment::Line {
                    start_mm: [95.0, 40.0],
                    end_mm: [95.0, 20.0],
                },
            ],
            [95.0, 20.0, 100.0, 40.0],
        ),
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [5.0, 20.0],
                    end_mm: [5.0, 40.0],
                    center_mm: [5.0, 30.0],
                    clockwise: true,
                },
                ProfileSegment::Line {
                    start_mm: [5.0, 40.0],
                    end_mm: [5.0, 20.0],
                },
            ],
            [0.0, 20.0, 5.0, 40.0],
        ),
    ];
    let expected_overlap_area = 50.0 * std::f64::consts::PI / 3.0 + 25.0 * 3.0_f64.sqrt();
    let expected_volume = 108_000.0 - expected_overlap_area * 8.0;
    let expected_topology = [16, 24, 10, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let planar_segments = planar_segments(segments.clone());
        let mut direct = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        capture_polygon_through_cut_references(
            &mut direct,
            "m163-document",
            "m163-pocket",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let mut document = mixed_profile_pocket_document(segments, 18.0, 8.0);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.d_profile_arc_only_clipped_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        for (actual, expected) in overlap_bounds.into_iter().zip(expected_overlap_bounds) {
            assert!((actual - expected).abs() < 1.0e-9);
        }
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
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
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let directory = tempfile::tempdir().unwrap();
        let step_path = directory
            .path()
            .join(format!("d-arc-side-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-d-arc-side-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
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
    }

    for segments in [
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 50.0],
                end_mm: [40.0, 50.0],
                center_mm: [30.0, 50.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 50.0],
                end_mm: [20.0, 50.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [20.0, 55.0],
                end_mm: [40.0, 55.0],
                center_mm: [30.0, 54.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [40.0, 55.0],
                end_mm: [20.0, 55.0],
            },
        ],
    ] {
        let rejected = mixed_profile_pocket_document(segments, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }

    let mut over_depth = rectangle_document(100.0, 60.0, 18.0);
    let before_revision = over_depth.current().revision_id();
    let before_digest = over_depth.current().canonical_digest();
    let before_undo = over_depth.visible_undo_steps();
    let result = over_depth.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateFeature {
            id: CUT_PROFILE,
            definition_id: DEFINITION,
            name: "Over-depth D profile".to_owned(),
            kind: FeatureKind::SegmentProfile {
                segments: vec![
                    ProfileSegment::CircularArc {
                        start_mm: [20.0, 55.0],
                        end_mm: [40.0, 55.0],
                        center_mm: [30.0, 55.0],
                        clockwise: true,
                    },
                    ProfileSegment::Line {
                        start_mm: [40.0, 55.0],
                        end_mm: [20.0, 55.0],
                    },
                ],
                closed: true,
            },
        },
        CanonicalCommand::CreateFeature {
            id: POCKET,
            definition_id: DEFINITION,
            name: "Over-depth D pocket".to_owned(),
            kind: FeatureKind::Pocket {
                target: EXTRUSION,
                profile: CUT_PROFILE,
                depth: Dimension::new("18", 18.0).unwrap(),
            },
        },
    ]));
    assert!(matches!(
        result,
        Err(CanonicalError::InvalidFeatureOwnership(POCKET))
    ));
    assert_eq!(over_depth.current().revision_id(), before_revision);
    assert_eq!(over_depth.current().canonical_digest(), before_digest);
    assert_eq!(over_depth.visible_undo_steps(), before_undo);
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
    let error = backend
        .cut_mixed_profile(&base.body, &concave_mixed_planar_segments(), -1.0, 20.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_side_overlapping_capsule_through_cut_and_step_round_trip() {
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
    let expected_overlap_area = 300.0 + 50.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;

    for (index, (dx, expected_overlap_bounds)) in [
        (55.0, [75.0, 20.0, 100.0, 40.0]),
        (-35.0, [0.0, 20.0, 25.0, 40.0]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Cut,
            translated_capsule_segments(dx, 0.0),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let cut = backend
            .cut_mixed_profile(
                &base.body,
                &translated_capsule_planar_segments(dx),
                -1.0,
                20.0,
            )
            .unwrap();
        assert_eq!(
            [
                cut.body.topology.vertex_count,
                cut.body.topology.edge_count,
                cut.body.topology.face_count,
                cut.body.topology.shell_count,
                cut.body.topology.solid_count,
            ],
            [16, 24, 10, 1, 1]
        );
        assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

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
                .any(|triangle| { triangle.face_role == Some(ExactFaceRole::CutLinear) })
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("side-overlap-capsule-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-side-overlap-capsule-cut-{index}.step"));
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
            [16, 24, 10, 1, 1]
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0003,
            "capsule side-overlap STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for (dx, dy) in [(50.0, 0.0), (70.0, 0.0), (0.0, -25.0)] {
        let rejected = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Cut,
            translated_capsule_segments(dx, dy),
        );
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_capsule_through_cut_and_step_round_trip() {
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
    let clipped_segment = radius * radius * (clip_distance / radius).acos()
        - clip_distance * (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_overlap_area =
        15.0 * 15.0 + 0.5 * (std::f64::consts::PI * radius * radius - clipped_segment);
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;
    let cases = vec![
        (
            translated_capsule_segments(55.0, -25.0),
            [75.0, 0.0, 100.0, 15.0],
        ),
        (
            translated_capsule_segments(55.0, 25.0),
            [75.0, 45.0, 100.0, 60.0],
        ),
        (
            translated_capsule_segments(-35.0, -25.0),
            [0.0, 0.0, 25.0, 15.0],
        ),
        (
            translated_capsule_segments(-35.0, 25.0),
            [0.0, 45.0, 25.0, 60.0],
        ),
        (
            rotated_capsule_segments(125.0, -35.0),
            [85.0, 0.0, 100.0, 25.0],
        ),
        (
            rotated_capsule_segments(125.0, 15.0),
            [85.0, 35.0, 100.0, 60.0],
        ),
        (
            rotated_capsule_segments(35.0, -35.0),
            [0.0, 0.0, 15.0, 25.0],
        ),
        (
            rotated_capsule_segments(35.0, 15.0),
            [0.0, 35.0, 15.0, 60.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_corner_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);
        assert!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
                .is_none()
        );

        let planar_segments = planar_segments(segments);
        let cut = backend
            .cut_mixed_profile(&base.body, &planar_segments, -1.0, 20.0)
            .unwrap();
        let topology = [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ];
        assert_eq!(topology, [12, 18, 8, 1, 1]);
        assert!(
            (cut.body.topology.volume_mm3 - expected_volume).abs() < 5.0e-2,
            "actual={}, expected={}, classified_area={overlap_area}",
            cut.body.topology.volume_mm3,
            expected_volume,
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            cut.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let wall = package.reference(ExactFaceRole::CutLinear).unwrap();
        assert_eq!(wall.profile_feature_id, CUT_PROFILE);
        assert!(wall.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::CutLinear))
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-capsule-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-capsule-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(imported_volume_error / expected_volume < 0.0003);
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

    let side_request = ExactFeatureChainRequest::from_snapshot(
        &capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Cut,
            translated_capsule_segments(55.0, 0.0),
        )
        .current(),
        DEFINITION,
    )
    .unwrap();
    let side_profile = side_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(side_profile.capsule_side_overlap(100.0, 60.0).is_some());
    assert_eq!(side_profile.capsule_corner_overlap(100.0, 60.0), None);

    for segments in [
        translated_capsule_segments(55.0, -20.0),
        translated_capsule_segments(55.0, -30.0),
        translated_capsule_segments(55.0, -45.0),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_capsule_pocket_and_step_round_trip() {
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
    let clipped_segment = radius * radius * (clip_distance / radius).acos()
        - clip_distance * (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_overlap_area =
        15.0 * 15.0 + 0.5 * (std::f64::consts::PI * radius * radius - clipped_segment);
    let expected_volume = 108_000.0 - expected_overlap_area * 8.0;
    let expected_topology = [14, 21, 9, 1, 1];
    let cases = vec![
        (
            translated_capsule_segments(55.0, -25.0),
            [75.0, 0.0, 100.0, 15.0],
        ),
        (
            translated_capsule_segments(55.0, 25.0),
            [75.0, 45.0, 100.0, 60.0],
        ),
        (
            translated_capsule_segments(-35.0, -25.0),
            [0.0, 0.0, 25.0, 15.0],
        ),
        (
            translated_capsule_segments(-35.0, 25.0),
            [0.0, 45.0, 25.0, 60.0],
        ),
        (
            rotated_capsule_segments(125.0, -35.0),
            [85.0, 0.0, 100.0, 25.0],
        ),
        (
            rotated_capsule_segments(125.0, 15.0),
            [85.0, 35.0, 100.0, 60.0],
        ),
        (
            rotated_capsule_segments(35.0, -35.0),
            [0.0, 0.0, 15.0, 25.0],
        ),
        (
            rotated_capsule_segments(35.0, 15.0),
            [0.0, 35.0, 15.0, 60.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document = mixed_profile_pocket_document(segments.clone(), 18.0, 8.0);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_corner_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);
        assert!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
                .is_none()
        );

        let planar_segments = planar_segments(segments);
        let mut pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        capture_polygon_through_cut_references(
            &mut pocket,
            "m157-document",
            "m157-pocket",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert_eq!(
            [
                pocket.body.topology.vertex_count,
                pocket.body.topology.edge_count,
                pocket.body.topology.face_count,
                pocket.body.topology.shell_count,
                pocket.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (pocket.body.topology.volume_mm3 - expected_volume).abs() < 5.0e-2,
            "actual={}, expected={}, classified_area={overlap_area}",
            pocket.body.topology.volume_mm3,
            expected_volume
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            pocket.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
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
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-capsule-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-capsule-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(imported_volume_error / expected_volume < 0.0003);
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

    for segments in [
        translated_capsule_segments(55.0, -20.0),
        translated_capsule_segments(55.0, -30.0),
        translated_capsule_segments(55.0, -45.0),
        rotated_capsule_segments(135.0, 15.0),
    ] {
        let rejected = mixed_profile_pocket_document(segments, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
    for pocket_depth in [18.0, 19.0] {
        let mut rejected = rectangle_document(100.0, 60.0, 18.0);
        let before_revision = rejected.current().revision_id();
        let before_digest = rejected.current().canonical_digest();
        let before_undo = rejected.visible_undo_steps();
        let result = rejected.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Over-depth corner capsule profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: translated_capsule_segments(55.0, -25.0),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Over-depth corner capsule pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::new(pocket_depth.to_string(), pocket_depth).unwrap(),
                },
            },
        ]));
        assert!(matches!(
            result,
            Err(CanonicalError::InvalidFeatureOwnership(POCKET))
        ));
        assert_eq!(rejected.current().revision_id(), before_revision);
        assert_eq!(rejected.current().canonical_digest(), before_digest);
        assert_eq!(rejected.visible_undo_steps(), before_undo);
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_capsule_intersect_and_step_round_trip() {
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
    let clipped_segment = radius * radius * (clip_distance / radius).acos()
        - clip_distance * (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_overlap_area =
        15.0 * 15.0 + 0.5 * (std::f64::consts::PI * radius * radius - clipped_segment);
    let expected_volume = expected_overlap_area * 18.0;
    let expected_topology = [8, 12, 6, 1, 1];
    let cases = vec![
        (
            translated_capsule_segments(55.0, -25.0),
            [75.0, 0.0, 100.0, 15.0],
        ),
        (
            translated_capsule_segments(55.0, 25.0),
            [75.0, 45.0, 100.0, 60.0],
        ),
        (
            translated_capsule_segments(-35.0, -25.0),
            [0.0, 0.0, 25.0, 15.0],
        ),
        (
            translated_capsule_segments(-35.0, 25.0),
            [0.0, 45.0, 25.0, 60.0],
        ),
        (
            rotated_capsule_segments(125.0, -35.0),
            [85.0, 0.0, 100.0, 25.0],
        ),
        (
            rotated_capsule_segments(125.0, 15.0),
            [85.0, 35.0, 100.0, 60.0],
        ),
        (
            rotated_capsule_segments(35.0, -35.0),
            [0.0, 0.0, 15.0, 25.0],
        ),
        (
            rotated_capsule_segments(35.0, 15.0),
            [0.0, 35.0, 15.0, 60.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            segments.clone(),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [expected_overlap_bounds[0], expected_overlap_bounds[1], 0.0],
                [expected_overlap_bounds[2], expected_overlap_bounds[3], 18.0],
            ]
        );
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, overlap_bounds) = profile.capsule_corner_overlap(100.0, 60.0).unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);
        assert_eq!(profile.capsule_side_overlap(100.0, 60.0), None);

        let intersection = backend
            .common_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                intersection.body.topology.vertex_count,
                intersection.body.topology.edge_count,
                intersection.body.topology.face_count,
                intersection.body.topology.shell_count,
                intersection.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (intersection.body.topology.volume_mm3 - expected_volume).abs() < 5.0e-2,
            "actual={}, expected={}, classified_area={overlap_area}",
            intersection.body.topology.volume_mm3,
            expected_volume
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
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-capsule-intersect-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-corner-overlap-capsule-intersect-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(imported_volume_error / expected_volume < 0.0032);
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

    let side_request = ExactFeatureChainRequest::from_snapshot(
        &capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_capsule_segments(55.0, 0.0),
        )
        .current(),
        DEFINITION,
    )
    .unwrap();
    let side_profile = side_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(side_profile.capsule_side_overlap(100.0, 60.0).is_some());
    assert_eq!(side_profile.capsule_corner_overlap(100.0, 60.0), None);

    for segments in [
        translated_capsule_segments(55.0, -20.0),
        translated_capsule_segments(55.0, -30.0),
        translated_capsule_segments(55.0, -45.0),
        rotated_capsule_segments(135.0, 15.0),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_capsule_union_and_step_round_trip() {
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
    let clipped_segment = radius * radius * (clip_distance / radius).acos()
        - clip_distance * (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_overlap_area =
        15.0 * 15.0 + 0.5 * (std::f64::consts::PI * radius * radius - clipped_segment);
    let capsule_area = 20.0 * 20.0 + std::f64::consts::PI * radius * radius;
    let expected_volume = (6_000.0 + capsule_area - expected_overlap_area) * 18.0;
    let expected_topology = [16, 24, 10, 1, 1];
    let cases = vec![
        translated_capsule_segments(55.0, -25.0),
        translated_capsule_segments(55.0, 25.0),
        translated_capsule_segments(-35.0, -25.0),
        translated_capsule_segments(-35.0, 25.0),
        rotated_capsule_segments(125.0, -35.0),
        rotated_capsule_segments(125.0, 15.0),
        rotated_capsule_segments(35.0, -35.0),
        rotated_capsule_segments(35.0, 15.0),
    ];

    for (index, segments) in cases.into_iter().enumerate() {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, overlap_bounds) = profile.capsule_corner_overlap(100.0, 60.0).unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(profile.capsule_side_overlap(100.0, 60.0), None);
        assert!(overlap_bounds[0] >= 0.0);
        assert!(overlap_bounds[1] >= 0.0);
        assert!(overlap_bounds[2] <= 100.0);
        assert!(overlap_bounds[3] <= 60.0);
        let [min_x, min_y, max_x, max_y] = profile.bounds_bits.map(f64::from_bits);
        let expected_bounds = [
            [min_x.min(0.0), min_y.min(0.0), 0.0],
            [max_x.max(100.0), max_y.max(60.0), 18.0],
        ];
        assert_eq!(request.expected_bounds_mm(), expected_bounds);

        let union = backend
            .fuse_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                union.body.topology.vertex_count,
                union.body.topology.edge_count,
                union.body.topology.face_count,
                union.body.topology.shell_count,
                union.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (union.body.topology.volume_mm3 - expected_volume).abs() < 5.0e-2,
            "actual={}, expected={}, classified_area={overlap_area}",
            union.body.topology.volume_mm3,
            expected_volume
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
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, expected_bounds);
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-capsule-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-capsule-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(imported_volume_error / expected_volume < 0.0032);
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

    let side_request = ExactFeatureChainRequest::from_snapshot(
        &capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_capsule_segments(55.0, 0.0),
        )
        .current(),
        DEFINITION,
    )
    .unwrap();
    let side_profile = side_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(side_profile.capsule_side_overlap(100.0, 60.0).is_some());
    assert_eq!(side_profile.capsule_corner_overlap(100.0, 60.0), None);

    for segments in [
        translated_capsule_segments(55.0, -20.0),
        translated_capsule_segments(55.0, -30.0),
        translated_capsule_segments(55.0, -45.0),
        rotated_capsule_segments(135.0, 15.0),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
    for operation in [
        BooleanOperation::Cut,
        BooleanOperation::Intersect,
        BooleanOperation::Split,
    ] {
        let request = ExactFeatureChainRequest::from_snapshot(
            &capsule_boolean_document_with_segments(
                18.0,
                operation,
                translated_capsule_segments(55.0, -25.0),
            )
            .current(),
            DEFINITION,
        )
        .unwrap();
        assert!(
            request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.capsule_corner_overlap(100.0, 60.0))
                .is_some()
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_capsule_split_and_step_round_trip() {
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
    let clipped_segment = radius * radius * (clip_distance / radius).acos()
        - clip_distance * (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_overlap_area =
        15.0 * 15.0 + 0.5 * (std::f64::consts::PI * radius * radius - clipped_segment);
    let expected_volume = 108_000.0;
    let expected_direct_topology = [14, 23, 12, 2, 2];
    let expected_step_topology = [20, 30, 14, 2, 2];
    let cases = vec![
        (
            translated_capsule_segments(55.0, -25.0),
            [75.0, 0.0, 100.0, 15.0],
        ),
        (
            translated_capsule_segments(55.0, 25.0),
            [75.0, 45.0, 100.0, 60.0],
        ),
        (
            translated_capsule_segments(-35.0, -25.0),
            [0.0, 0.0, 25.0, 15.0],
        ),
        (
            translated_capsule_segments(-35.0, 25.0),
            [0.0, 45.0, 25.0, 60.0],
        ),
        (
            rotated_capsule_segments(125.0, -35.0),
            [85.0, 0.0, 100.0, 25.0],
        ),
        (
            rotated_capsule_segments(125.0, 15.0),
            [85.0, 35.0, 100.0, 60.0],
        ),
        (
            rotated_capsule_segments(35.0, -35.0),
            [0.0, 0.0, 15.0, 25.0],
        ),
        (
            rotated_capsule_segments(35.0, 15.0),
            [0.0, 35.0, 15.0, 60.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_corner_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let split = backend
            .split_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                split.body.topology.vertex_count,
                split.body.topology.edge_count,
                split.body.topology.face_count,
                split.body.topology.shell_count,
                split.body.topology.solid_count,
            ],
            expected_direct_topology
        );
        assert!((split.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 2);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-capsule-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-capsule-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology
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
    }

    let side_request = ExactFeatureChainRequest::from_snapshot(
        &capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Split,
            translated_capsule_segments(55.0, 0.0),
        )
        .current(),
        DEFINITION,
    )
    .unwrap();
    let side_profile = side_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(side_profile.capsule_side_overlap(100.0, 60.0).is_some());
    assert_eq!(side_profile.capsule_corner_overlap(100.0, 60.0), None);

    for segments in [
        translated_capsule_segments(55.0, -20.0),
        translated_capsule_segments(55.0, -30.0),
        translated_capsule_segments(55.0, -45.0),
        rotated_capsule_segments(135.0, 15.0),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_north_south_side_overlapping_capsule_through_cut() {
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
    let expected_overlap_area = 300.0 + 50.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;

    for (index, (dy, expected_overlap_bounds)) in [
        (15.0, [40.0, 35.0, 60.0, 60.0]),
        (-35.0, [40.0, 0.0, 60.0, 25.0]),
    ]
    .into_iter()
    .enumerate()
    {
        let segments = rotated_capsule_segments(80.0, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let cut = backend
            .cut_mixed_profile(&base.body, &planar_segments(segments), -1.0, 20.0)
            .unwrap();
        assert_eq!(
            [
                cut.body.topology.vertex_count,
                cut.body.topology.edge_count,
                cut.body.topology.face_count,
                cut.body.topology.shell_count,
                cut.body.topology.solid_count,
            ],
            [16, 24, 10, 1, 1]
        );
        assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

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
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("north-south-capsule-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-north-south-capsule-cut-{index}.step"));
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
            [16, 24, 10, 1, 1]
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(imported_volume_error / expected_volume < 0.0003);
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
        rotated_capsule_segments(80.0, 40.0),
        rotated_capsule_segments(80.0, 50.0),
        rotated_capsule_segments(135.0, 15.0),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected_segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_side_overlapping_capsule_pocket_and_step_round_trip() {
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
    let expected_overlap_area = 300.0 + 50.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0 - expected_overlap_area * 8.0;
    let expected_topology = [16, 24, 10, 1, 1];
    let cases = vec![
        (
            translated_capsule_segments(55.0, 0.0),
            [75.0, 20.0, 100.0, 40.0],
        ),
        (
            translated_capsule_segments(-35.0, 0.0),
            [0.0, 20.0, 25.0, 40.0],
        ),
        (
            rotated_capsule_segments(80.0, 15.0),
            [40.0, 35.0, 60.0, 60.0],
        ),
        (
            rotated_capsule_segments(80.0, -35.0),
            [40.0, 0.0, 60.0, 25.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document = mixed_profile_pocket_document(segments.clone(), 18.0, 8.0);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let planar_segments = planar_segments(segments);
        let mut pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        capture_polygon_through_cut_references(
            &mut pocket,
            "m150-document",
            "m150-pocket",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert_eq!(
            [
                pocket.body.topology.vertex_count,
                pocket.body.topology.edge_count,
                pocket.body.topology.face_count,
                pocket.body.topology.shell_count,
                pocket.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!((pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            pocket.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
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
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("side-overlap-capsule-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-side-overlap-capsule-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0003,
            "capsule pocket STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_capsule_segments(50.0, 0.0),
        translated_capsule_segments(70.0, 0.0),
        rotated_capsule_segments(80.0, 40.0),
        rotated_capsule_segments(135.0, 15.0),
        concave_mixed_segments(),
    ] {
        let rejected = mixed_profile_pocket_document(rejected_segments, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_side_overlapping_capsule_union_and_step_round_trip() {
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
    let expected_overlap_area = 300.0 + 50.0 * std::f64::consts::PI;
    let expected_volume =
        (6_000.0 + 400.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;
    let cases = vec![
        (
            translated_capsule_segments(55.0, 0.0),
            [75.0, 20.0, 100.0, 40.0],
            [[0.0, 0.0, 0.0], [115.0, 60.0, 18.0]],
        ),
        (
            translated_capsule_segments(-35.0, 0.0),
            [0.0, 20.0, 25.0, 40.0],
            [[-15.0, 0.0, 0.0], [100.0, 60.0, 18.0]],
        ),
        (
            rotated_capsule_segments(80.0, 15.0),
            [40.0, 35.0, 60.0, 60.0],
            [[0.0, 0.0, 0.0], [100.0, 75.0, 18.0]],
        ),
        (
            rotated_capsule_segments(80.0, -35.0),
            [40.0, 0.0, 60.0, 25.0],
            [[0.0, -15.0, 0.0], [100.0, 60.0, 18.0]],
        ),
    ];

    for (index, (segments, expected_overlap_bounds, expected_bounds)) in
        cases.into_iter().enumerate()
    {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let union = backend
            .fuse_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                union.body.topology.vertex_count,
                union.body.topology.edge_count,
                union.body.topology.face_count,
                union.body.topology.shell_count,
                union.body.topology.solid_count,
            ],
            [16, 24, 10, 1, 1]
        );
        assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            union.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("side-overlap-capsule-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-side-overlap-capsule-union-{index}.step"));
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
            [16, 24, 10, 1, 1]
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0006,
            "capsule union STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_capsule_segments(50.0, 0.0),
        translated_capsule_segments(70.0, 0.0),
        rotated_capsule_segments(80.0, 40.0),
        rotated_capsule_segments(135.0, 15.0),
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
fn scheduler_evaluates_side_overlapping_capsule_intersect_and_step_round_trip() {
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
    let expected_overlap_area = 300.0 + 50.0 * std::f64::consts::PI;
    let expected_volume = expected_overlap_area * 18.0;
    let expected_topology = [8, 12, 6, 1, 1];
    let cases = vec![
        (
            translated_capsule_segments(55.0, 0.0),
            [75.0, 20.0, 100.0, 40.0],
        ),
        (
            translated_capsule_segments(-35.0, 0.0),
            [0.0, 20.0, 25.0, 40.0],
        ),
        (
            rotated_capsule_segments(80.0, 15.0),
            [40.0, 35.0, 60.0, 60.0],
        ),
        (
            rotated_capsule_segments(80.0, -35.0),
            [40.0, 0.0, 60.0, 25.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            segments.clone(),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [expected_overlap_bounds[0], expected_overlap_bounds[1], 0.0,],
                [expected_overlap_bounds[2], expected_overlap_bounds[3], 18.0,],
            ]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let intersection = backend
            .common_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                intersection.body.topology.vertex_count,
                intersection.body.topology.edge_count,
                intersection.body.topology.face_count,
                intersection.body.topology.shell_count,
                intersection.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            intersection.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("side-overlap-capsule-intersect-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-side-overlap-capsule-intersect-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0032,
            "capsule intersection STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_capsule_segments(50.0, 0.0),
        translated_capsule_segments(70.0, 0.0),
        rotated_capsule_segments(80.0, 40.0),
        rotated_capsule_segments(135.0, 15.0),
        concave_mixed_segments(),
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
fn scheduler_evaluates_side_overlapping_capsule_split_and_step_round_trip() {
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
    let expected_overlap_area = 300.0 + 50.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0;
    let expected_direct_topology = [16, 26, 13, 2, 2];
    let expected_step_topology = [24, 36, 16, 2, 2];
    let cases = vec![
        (
            translated_capsule_segments(55.0, 0.0),
            [75.0, 20.0, 100.0, 40.0],
        ),
        (
            translated_capsule_segments(-35.0, 0.0),
            [0.0, 20.0, 25.0, 40.0],
        ),
        (
            rotated_capsule_segments(80.0, 15.0),
            [40.0, 35.0, 60.0, 60.0],
        ),
        (
            rotated_capsule_segments(80.0, -35.0),
            [40.0, 0.0, 60.0, 25.0],
        ),
    ];

    for (index, (segments, expected_overlap_bounds)) in cases.into_iter().enumerate() {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, overlap_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.capsule_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(overlap_bounds, expected_overlap_bounds);

        let split = backend
            .split_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                split.body.topology.vertex_count,
                split.body.topology.edge_count,
                split.body.topology.face_count,
                split.body.topology.shell_count,
                split.body.topology.solid_count,
            ],
            expected_direct_topology
        );
        assert!((split.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            split.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 2);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("side-overlap-capsule-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-side-overlap-capsule-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0003,
            "capsule split STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_capsule_segments(50.0, 0.0),
        translated_capsule_segments(70.0, 0.0),
        rotated_capsule_segments(80.0, 40.0),
        rotated_capsule_segments(135.0, 15.0),
        concave_mixed_segments(),
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
fn scheduler_evaluates_asymmetric_strict_convex_mixed_through_cut_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_strict_convex_line_arc_profile());
    assert!(!profile.is_line_arc_d_profile());
    assert!(!profile.is_line_arc_capsule_profile());
    assert!(!profile.is_line_arc_rounded_rectangle_profile());

    let profile_area = 1_300.0 + 25.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0 - profile_area * 18.0;
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(
            &base.body,
            &asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            -1.0,
            20.0,
        )
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let package = ExactWorkerSupervisor::spawn(worker_path())
        .unwrap()
        .evaluate_rectangle(&request)
        .unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        cut.body.result_fingerprint
    );
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

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("asymmetric-convex-mixed-cut.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

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
        .join("stale-asymmetric-convex-mixed-cut.step");
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

    let rejected_union =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected_union.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Union
        ))
    );
    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
    let error = backend
        .cut_mixed_profile(&base.body, &concave_mixed_planar_segments(), -1.0, 20.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_contained_asymmetric_strict_convex_mixed_pocket_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document = mixed_profile_pocket_document(segments, 18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_strict_convex_line_arc_profile());
    assert!(!profile.is_line_arc_d_profile());
    assert!(!profile.is_line_arc_capsule_profile());
    assert!(!profile.is_line_arc_rounded_rectangle_profile());
    assert_eq!(
        profile.bounds_bits.map(f64::from_bits),
        [25.0, 15.0, 75.0, 45.0]
    );
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_line_arc_clipped_side_overlap(100.0, 60.0),
        None
    );
    let profile_area = 1_300.0 + 25.0 * std::f64::consts::PI;
    let expected_volume = 108_000.0 - profile_area * 8.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let planar_segments = asymmetric_convex_mixed_planar_segments(0.0, 0.0);
    let mut pocket = backend
        .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut pocket,
        "m119-document",
        "m119-pocket",
        Some(10.0),
        Some(&planar_segments),
    )
    .unwrap();
    assert!((pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            pocket.body.topology.vertex_count,
            pocket.body.topology.edge_count,
            pocket.body.topology.face_count,
            pocket.body.topology.shell_count,
            pocket.body.topology.solid_count,
        ],
        [18, 27, 12, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        pocket.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::PocketFloor))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("contained-asymmetric-convex-mixed-pocket.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [18, 27, 12, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-contained-asymmetric-convex-mixed-pocket.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        asymmetric_convex_mixed_segments(-76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected = mixed_profile_pocket_document(rejected, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_clipped_through_cut_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(-30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let overlap_area = 1_150.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [0.0, 15.0, 45.0, 45.0]))
    );
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(
            &base.body,
            &asymmetric_convex_mixed_planar_segments(-30.0, 0.0),
            -1.0,
            20.0,
        )
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-clipped-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-clipped-cut.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        asymmetric_convex_mixed_segments(35.0, 0.0),
        asymmetric_convex_mixed_segments(-30.0, -20.0),
        asymmetric_convex_mixed_segments(-76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_clipped_pocket_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(-30.0, 0.0);
    let mut document = mixed_profile_pocket_document(segments, 18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let overlap_area = 1_150.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [0.0, 15.0, 45.0, 45.0]))
    );
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_line_arc_clipped_side_overlap(100.0, 60.0),
        None
    );
    let expected_volume = 108_000.0 - overlap_area * 8.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let planar_segments = asymmetric_convex_mixed_planar_segments(-30.0, 0.0);
    let mut pocket = backend
        .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut pocket,
        "m118-document",
        "m118-pocket",
        Some(10.0),
        Some(&planar_segments),
    )
    .unwrap();
    assert!((pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            pocket.body.topology.vertex_count,
            pocket.body.topology.edge_count,
            pocket.body.topology.face_count,
            pocket.body.topology.shell_count,
            pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        pocket.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::PocketFloor))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-clipped-pocket.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-clipped-pocket.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        asymmetric_convex_mixed_segments(-30.0, -20.0),
        asymmetric_convex_mixed_segments(-76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected = mixed_profile_pocket_document(rejected, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_arc_only_clipped_through_cut_end_to_end() {
    let segments = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let distance = 10.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [30.0, 15.0, 100.0, 45.0]))
    );
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(
            &base.body,
            &arc_only_clipped_asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            -1.0,
            20.0,
        )
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [22, 33, 13, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-arc-only-clipped-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-arc-only-clipped-cut.step");
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0),
        asymmetric_convex_mixed_segments(35.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_arc_clipped_corner_cut_and_pocket_end_to_end()
{
    let mut segments = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    segments.rotate_left(2);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [30.0, 0.0, 100.0, 25.0]);
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(&base.body, &planar_segments(segments.clone()), -1.0, 20.0)
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [16, 24, 10, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("asymmetric-convex-corner-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-corner-cut.step");
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

    let mut pocket_document = mixed_profile_pocket_document(segments.clone(), 18.0, 8.0);
    let pocket_snapshot = pocket_document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let expected_pocket_volume = 108_000.0 - overlap_area * 8.0;
    let pocket_segments = planar_segments(segments.clone());
    let mut native_pocket = backend
        .cut_mixed_profile(&base.body, &pocket_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut native_pocket,
        "m121-document",
        "m121-pocket",
        Some(10.0),
        Some(&pocket_segments),
    )
    .unwrap();
    assert!((native_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            native_pocket.body.topology.vertex_count,
            native_pocket.body.topology.edge_count,
            native_pocket.body.topology.face_count,
            native_pocket.body.topology.shell_count,
            native_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    let repeated_pocket = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    assert!(pocket_package.is_current(&pocket_snapshot));
    assert_eq!(
        pocket_package.identity.result_fingerprint,
        native_pocket.body.result_fingerprint
    );
    assert_eq!(pocket_package.identity, repeated_pocket.identity);
    assert_eq!(pocket_package.references, repeated_pocket.references);
    assert_eq!(
        pocket_package.bounds_mm,
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = pocket_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(pocket_package.triangles.iter().any(|triangle| {
        triangle.face_role == Some(ExactFaceRole::PocketFloor)
            && triangle.vertex_indices.iter().all(|index| {
                (pocket_package.vertices[*index as usize].position_mm[2] - 10.0).abs() <= 1.0e-9
            })
    }));
    assert!(pocket_package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
            && vertex.position_mm[2] >= -1.0e-9
            && vertex.position_mm[2] <= 18.0 + 1.0e-9
    }));
    assert_closed_manifold(&pocket_package);
    assert_eq!(mesh_component_count(&pocket_package), 1);

    let pocket_step_path = directory
        .path()
        .join("asymmetric-convex-corner-pocket.step");
    let pocket_model = [(
        ExactBodyPackage::from(pocket_package.clone()),
        Transform::identity(),
    )];
    let pocket_before_revision = pocket_snapshot.revision_id();
    let pocket_before_digest = pocket_snapshot.canonical_digest();
    let pocket_before_undo = pocket_document.visible_undo_steps();
    supervisor
        .export_current_model_step(&pocket_snapshot, &pocket_model, &pocket_step_path)
        .unwrap();
    let imported_pocket = backend
        .import_step(pocket_step_path.to_str().unwrap())
        .unwrap();
    assert!((imported_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported_pocket.body.topology.vertex_count,
            imported_pocket.body.topology.edge_count,
            imported_pocket.body.topology.face_count,
            imported_pocket.body.topology.shell_count,
            imported_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );
    assert_eq!(
        pocket_document.current().revision_id(),
        pocket_before_revision
    );
    assert_eq!(
        pocket_document.current().canonical_digest(),
        pocket_before_digest
    );
    assert_eq!(pocket_document.visible_undo_steps(), pocket_before_undo);

    pocket_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    let stale_pocket_path = directory
        .path()
        .join("stale-asymmetric-convex-corner-pocket.step");
    std::fs::write(&stale_pocket_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(
                &pocket_document.current(),
                &pocket_model,
                &stale_pocket_path,
            )
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_pocket_path).unwrap(),
        b"preserved destination"
    );
    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, -20.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected_pocket = mixed_profile_pocket_document(rejected.clone(), 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected_pocket.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_mirrored_asymmetric_strict_convex_corner_cut_and_pocket_end_to_end() {
    let mut southeast = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    southeast.rotate_left(2);
    let mirror = |point: [f64; 2]| [100.0 - point[0], point[1]];
    let segments = southeast
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                start_mm: mirror(start_mm),
                end_mm: mirror(end_mm),
            },
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => ProfileSegment::CircularArc {
                start_mm: mirror(start_mm),
                end_mm: mirror(end_mm),
                center_mm: mirror(center_mm),
                clockwise: !clockwise,
            },
        })
        .collect::<Vec<_>>();
    let document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_south_west_corner_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [0.0, 0.0, 70.0, 25.0]);
    assert_eq!(
        profile.strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0),
        None
    );
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let native_segments = planar_segments(segments.clone());
    let cut = backend
        .cut_mixed_profile(&base.body, &native_segments, -1.0, 20.0)
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [16, 24, 10, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("mirrored-asymmetric-corner-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.current().canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo);

    let mut pocket_document = mixed_profile_pocket_document(segments.clone(), 18.0, 8.0);
    let pocket_snapshot = pocket_document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    let expected_pocket_volume = 108_000.0 - overlap_area * 8.0;
    let mut native_pocket = backend
        .cut_mixed_profile(&base.body, &native_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut native_pocket,
        "m122-document",
        "m122-pocket",
        Some(10.0),
        Some(&native_segments),
    )
    .unwrap();
    assert!((native_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            native_pocket.body.topology.vertex_count,
            native_pocket.body.topology.edge_count,
            native_pocket.body.topology.face_count,
            native_pocket.body.topology.shell_count,
            native_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    let repeated_pocket = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    assert!(pocket_package.is_current(&pocket_snapshot));
    assert_eq!(
        pocket_package.identity.result_fingerprint,
        native_pocket.body.result_fingerprint
    );
    assert_eq!(pocket_package.identity, repeated_pocket.identity);
    assert_eq!(pocket_package.references, repeated_pocket.references);
    assert_eq!(
        pocket_package.bounds_mm,
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = pocket_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(pocket_package.triangles.iter().any(|triangle| {
        triangle.face_role == Some(ExactFaceRole::PocketFloor)
            && triangle.vertex_indices.iter().all(|index| {
                (pocket_package.vertices[*index as usize].position_mm[2] - 10.0).abs() <= 1.0e-9
            })
    }));
    assert!(pocket_package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
            && vertex.position_mm[2] >= -1.0e-9
            && vertex.position_mm[2] <= 18.0 + 1.0e-9
    }));
    assert_closed_manifold(&pocket_package);
    assert_eq!(mesh_component_count(&pocket_package), 1);

    let pocket_step_path = directory
        .path()
        .join("mirrored-asymmetric-corner-pocket.step");
    let pocket_model = [(
        ExactBodyPackage::from(pocket_package.clone()),
        Transform::identity(),
    )];
    let pocket_before_revision = pocket_snapshot.revision_id();
    let pocket_before_digest = pocket_snapshot.canonical_digest();
    let pocket_before_undo = pocket_document.visible_undo_steps();
    supervisor
        .export_current_model_step(&pocket_snapshot, &pocket_model, &pocket_step_path)
        .unwrap();
    let imported_pocket = backend
        .import_step(pocket_step_path.to_str().unwrap())
        .unwrap();
    assert!((imported_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported_pocket.body.topology.vertex_count,
            imported_pocket.body.topology.edge_count,
            imported_pocket.body.topology.face_count,
            imported_pocket.body.topology.shell_count,
            imported_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );
    assert_eq!(
        pocket_document.current().revision_id(),
        pocket_before_revision
    );
    assert_eq!(
        pocket_document.current().canonical_digest(),
        pocket_before_digest
    );
    assert_eq!(pocket_document.visible_undo_steps(), pocket_before_undo);

    pocket_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::new("24", 24.0).unwrap(),
            },
        ]))
        .unwrap();
    let stale_path = directory.path().join("stale-mirrored-corner-pocket.step");
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        supervisor
            .export_current_model_step(&pocket_document.current(), &pocket_model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );

    let mut north_west = segments;
    for segment in &mut north_west {
        match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                start_mm[1] += 40.0;
                end_mm[1] += 40.0;
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                ..
            } => {
                start_mm[1] += 40.0;
                end_mm[1] += 40.0;
                center_mm[1] += 40.0;
            }
        }
    }
    let rejected = mixed_profile_pocket_document(north_west, 18.0, 8.0);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
    );
}

#[test]
fn scheduler_evaluates_north_east_mirrored_corner_cut_and_pocket_end_to_end() {
    let mut southeast = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    southeast.rotate_left(2);
    let mirror = |point: [f64; 2]| [point[0], 60.0 - point[1]];
    let segments = southeast
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                start_mm: mirror(start_mm),
                end_mm: mirror(end_mm),
            },
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => ProfileSegment::CircularArc {
                start_mm: mirror(start_mm),
                end_mm: mirror(end_mm),
                center_mm: mirror(center_mm),
                clockwise: !clockwise,
            },
        })
        .collect::<Vec<_>>();
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_north_east_corner_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [30.0, 35.0, 100.0, 60.0]);
    assert_eq!(
        profile.strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_line_arc_clipped_south_west_corner_overlap(100.0, 60.0),
        None
    );
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let native_segments = planar_segments(segments.clone());
    let cut = backend
        .cut_mixed_profile(&base.body, &native_segments, -1.0, 20.0)
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [16, 24, 10, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("north-east-asymmetric-corner-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory.path().join("stale-north-east-corner-cut.step");
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

    let pocket_document = mixed_profile_pocket_document(segments.clone(), 18.0, 8.0);
    let pocket_snapshot = pocket_document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let expected_pocket_volume = 108_000.0 - overlap_area * 8.0;
    let mut native_pocket = backend
        .cut_mixed_profile(&base.body, &native_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut native_pocket,
        "m123-document",
        "m123-pocket",
        Some(10.0),
        Some(&native_segments),
    )
    .unwrap();
    assert!((native_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            native_pocket.body.topology.vertex_count,
            native_pocket.body.topology.edge_count,
            native_pocket.body.topology.face_count,
            native_pocket.body.topology.shell_count,
            native_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    let repeated_pocket = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    assert!(pocket_package.is_current(&pocket_snapshot));
    assert_eq!(
        pocket_package.identity.result_fingerprint,
        native_pocket.body.result_fingerprint
    );
    assert_eq!(pocket_package.identity, repeated_pocket.identity);
    assert_eq!(pocket_package.references, repeated_pocket.references);
    assert_eq!(
        pocket_package.bounds_mm,
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = pocket_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(pocket_package.triangles.iter().any(|triangle| {
        triangle.face_role == Some(ExactFaceRole::PocketFloor)
            && triangle.vertex_indices.iter().all(|index| {
                (pocket_package.vertices[*index as usize].position_mm[2] - 10.0).abs() <= 1.0e-9
            })
    }));
    assert!(pocket_package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
            && vertex.position_mm[2] >= -1.0e-9
            && vertex.position_mm[2] <= 18.0 + 1.0e-9
    }));
    assert_closed_manifold(&pocket_package);
    assert_eq!(mesh_component_count(&pocket_package), 1);

    let pocket_step_path = directory
        .path()
        .join("north-east-asymmetric-corner-pocket.step");
    let pocket_model = [(
        ExactBodyPackage::from(pocket_package.clone()),
        Transform::identity(),
    )];
    let pocket_before_revision = pocket_snapshot.revision_id();
    let pocket_before_digest = pocket_snapshot.canonical_digest();
    let pocket_before_undo = pocket_document.visible_undo_steps();
    supervisor
        .export_current_model_step(&pocket_snapshot, &pocket_model, &pocket_step_path)
        .unwrap();
    let imported_pocket = backend
        .import_step(pocket_step_path.to_str().unwrap())
        .unwrap();
    assert!((imported_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported_pocket.body.topology.vertex_count,
            imported_pocket.body.topology.edge_count,
            imported_pocket.body.topology.face_count,
            imported_pocket.body.topology.shell_count,
            imported_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );
    assert_eq!(
        pocket_document.current().revision_id(),
        pocket_before_revision
    );
    assert_eq!(
        pocket_document.current().canonical_digest(),
        pocket_before_digest
    );
    assert_eq!(pocket_document.visible_undo_steps(), pocket_before_undo);

    for rejected in [concave_mixed_segments(), self_intersecting_mixed_segments()] {
        let rejected_pocket = mixed_profile_pocket_document(rejected, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected_pocket.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_north_west_mirrored_corner_cut_and_pocket_end_to_end() {
    let mut southeast = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    southeast.rotate_left(2);
    let mirror = |point: [f64; 2]| [100.0 - point[0], 60.0 - point[1]];
    let segments = southeast
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                start_mm: mirror(start_mm),
                end_mm: mirror(end_mm),
            },
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => ProfileSegment::CircularArc {
                start_mm: mirror(start_mm),
                end_mm: mirror(end_mm),
                center_mm: mirror(center_mm),
                clockwise,
            },
        })
        .collect::<Vec<_>>();
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_north_west_corner_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [0.0, 35.0, 70.0, 60.0]);
    assert_eq!(
        profile.strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_line_arc_clipped_south_west_corner_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_line_arc_clipped_north_east_corner_overlap(100.0, 60.0),
        None
    );
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let native_segments = planar_segments(segments.clone());
    let cut = backend
        .cut_mixed_profile(&base.body, &native_segments, -1.0, 20.0)
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [16, 24, 10, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("north-west-asymmetric-corner-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory.path().join("stale-north-west-corner-cut.step");
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

    let pocket_document = mixed_profile_pocket_document(segments.clone(), 18.0, 8.0);
    let pocket_snapshot = pocket_document.current();
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let expected_pocket_volume = 108_000.0 - overlap_area * 8.0;
    let mut native_pocket = backend
        .cut_mixed_profile(&base.body, &native_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut native_pocket,
        "m124-document",
        "m124-pocket",
        Some(10.0),
        Some(&native_segments),
    )
    .unwrap();
    assert!((native_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            native_pocket.body.topology.vertex_count,
            native_pocket.body.topology.edge_count,
            native_pocket.body.topology.face_count,
            native_pocket.body.topology.shell_count,
            native_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );

    let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    let repeated_pocket = supervisor.evaluate_rectangle(&pocket_request).unwrap();
    assert!(pocket_package.is_current(&pocket_snapshot));
    assert_eq!(
        pocket_package.identity.result_fingerprint,
        native_pocket.body.result_fingerprint
    );
    assert_eq!(pocket_package.identity, repeated_pocket.identity);
    assert_eq!(pocket_package.references, repeated_pocket.references);
    assert_eq!(
        pocket_package.bounds_mm,
        [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = pocket_package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(pocket_package.triangles.iter().any(|triangle| {
        triangle.face_role == Some(ExactFaceRole::PocketFloor)
            && triangle.vertex_indices.iter().all(|index| {
                (pocket_package.vertices[*index as usize].position_mm[2] - 10.0).abs() <= 1.0e-9
            })
    }));
    assert!(pocket_package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
            && vertex.position_mm[2] >= -1.0e-9
            && vertex.position_mm[2] <= 18.0 + 1.0e-9
    }));
    assert_closed_manifold(&pocket_package);
    assert_eq!(mesh_component_count(&pocket_package), 1);

    let pocket_step_path = directory
        .path()
        .join("north-west-asymmetric-corner-pocket.step");
    let pocket_model = [(
        ExactBodyPackage::from(pocket_package.clone()),
        Transform::identity(),
    )];
    let pocket_before_revision = pocket_snapshot.revision_id();
    let pocket_before_digest = pocket_snapshot.canonical_digest();
    let pocket_before_undo = pocket_document.visible_undo_steps();
    supervisor
        .export_current_model_step(&pocket_snapshot, &pocket_model, &pocket_step_path)
        .unwrap();
    let imported_pocket = backend
        .import_step(pocket_step_path.to_str().unwrap())
        .unwrap();
    assert!((imported_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported_pocket.body.topology.vertex_count,
            imported_pocket.body.topology.edge_count,
            imported_pocket.body.topology.face_count,
            imported_pocket.body.topology.shell_count,
            imported_pocket.body.topology.solid_count,
        ],
        [18, 27, 11, 1, 1]
    );
    assert_eq!(
        pocket_document.current().revision_id(),
        pocket_before_revision
    );
    assert_eq!(
        pocket_document.current().canonical_digest(),
        pocket_before_digest
    );
    assert_eq!(pocket_document.visible_undo_steps(), pocket_before_undo);

    for rejected in [concave_mixed_segments(), self_intersecting_mixed_segments()] {
        let rejected_pocket = mixed_profile_pocket_document(rejected, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected_pocket.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_four_asymmetric_strict_convex_corner_intersections_end_to_end() {
    let mut south_east = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    south_east.rotate_left(2);
    let mirror_segments = |segments: &[ProfileSegment], flip_x: bool, flip_y: bool| {
        segments
            .iter()
            .map(|segment| {
                let mirror = |[x, y]: [f64; 2]| {
                    [
                        if flip_x { 100.0 - x } else { x },
                        if flip_y { 60.0 - y } else { y },
                    ]
                };
                match segment {
                    ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                        start_mm: mirror(*start_mm),
                        end_mm: mirror(*end_mm),
                    },
                    ProfileSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => ProfileSegment::CircularArc {
                        start_mm: mirror(*start_mm),
                        end_mm: mirror(*end_mm),
                        center_mm: mirror(*center_mm),
                        clockwise: if flip_x ^ flip_y {
                            !*clockwise
                        } else {
                            *clockwise
                        },
                    },
                }
            })
            .collect::<Vec<_>>()
    };
    let cases = [
        (
            "south-east",
            south_east.clone(),
            [30.0, 0.0, 100.0, 25.0],
            0_usize,
        ),
        (
            "south-west",
            mirror_segments(&south_east, true, false),
            [0.0, 0.0, 70.0, 25.0],
            1,
        ),
        (
            "north-east",
            mirror_segments(&south_east, false, true),
            [30.0, 35.0, 100.0, 60.0],
            2,
        ),
        (
            "north-west",
            mirror_segments(&south_east, true, true),
            [0.0, 35.0, 70.0, 60.0],
            3,
        ),
    ];
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    let expected_volume = overlap_area * 18.0;
    let expected_topology = [12, 18, 8, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (name, segments, expected_bounds, classifier_index) in cases {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            segments.clone(),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let classifications = [
            profile.strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_south_west_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_north_east_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_north_west_corner_overlap(100.0, 60.0),
        ];
        assert_eq!(
            classifications
                .iter()
                .filter(|value| value.is_some())
                .count(),
            1
        );
        let (classified_area, classified_bounds) = classifications[classifier_index].unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, expected_bounds);

        let intersection = backend
            .common_mixed_profile(&base.body, &planar_segments(segments.clone()), 0.0, 18.0)
            .unwrap();
        assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                intersection.body.topology.vertex_count,
                intersection.body.topology.edge_count,
                intersection.body.topology.face_count,
                intersection.body.topology.shell_count,
                intersection.body.topology.solid_count,
            ],
            expected_topology
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
                [expected_bounds[0], expected_bounds[1], 0.0],
                [expected_bounds[2], expected_bounds[3], 18.0],
            ]
        );
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[2] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[3] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory.path().join(format!("m125-{name}-intersect.step"));
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
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
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
        let stale_path = directory
            .path()
            .join(format!("stale-m125-{name}-intersect.step"));
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, -20.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_four_asymmetric_strict_convex_corner_splits_end_to_end() {
    let mut south_east = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    south_east.rotate_left(2);
    let mirror_segments = |segments: &[ProfileSegment], flip_x: bool, flip_y: bool| {
        segments
            .iter()
            .map(|segment| {
                let mirror = |[x, y]: [f64; 2]| {
                    [
                        if flip_x { 100.0 - x } else { x },
                        if flip_y { 60.0 - y } else { y },
                    ]
                };
                match segment {
                    ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                        start_mm: mirror(*start_mm),
                        end_mm: mirror(*end_mm),
                    },
                    ProfileSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => ProfileSegment::CircularArc {
                        start_mm: mirror(*start_mm),
                        end_mm: mirror(*end_mm),
                        center_mm: mirror(*center_mm),
                        clockwise: if flip_x ^ flip_y {
                            !*clockwise
                        } else {
                            *clockwise
                        },
                    },
                }
            })
            .collect::<Vec<_>>()
    };
    let cases = [
        (
            "south-east",
            south_east.clone(),
            [30.0, 0.0, 100.0, 25.0],
            0_usize,
        ),
        (
            "south-west",
            mirror_segments(&south_east, true, false),
            [0.0, 0.0, 70.0, 25.0],
            1,
        ),
        (
            "north-east",
            mirror_segments(&south_east, false, true),
            [30.0, 35.0, 100.0, 60.0],
            2,
        ),
        (
            "north-west",
            mirror_segments(&south_east, true, true),
            [0.0, 35.0, 70.0, 60.0],
            3,
        ),
    ];
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    let expected_topology = [18, 29, 14, 2, 2];
    let expected_step_topology = [28, 42, 18, 2, 2];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (name, segments, expected_overlap_bounds, classifier_index) in cases {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let classifications = [
            profile.strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_south_west_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_north_east_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_north_west_corner_overlap(100.0, 60.0),
        ];
        assert_eq!(
            classifications
                .iter()
                .filter(|classification| classification.is_some())
                .count(),
            1
        );
        let (classified_area, classified_bounds) = classifications[classifier_index].unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, expected_overlap_bounds);

        let split = backend
            .split_mixed_profile(&base.body, &planar_segments(segments.clone()), 0.0, 18.0)
            .unwrap();
        assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
        assert_eq!(
            [
                split.body.topology.vertex_count,
                split.body.topology.edge_count,
                split.body.topology.face_count,
                split.body.topology.shell_count,
                split.body.topology.solid_count,
            ],
            expected_topology
        );

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
        assert_eq!((package.vertices.len(), package.triangles.len()), (56, 104));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory.path().join(format!("m126-{name}-split.step"));
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
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology
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
        let stale_path = directory
            .path()
            .join(format!("stale-m126-{name}-split.step"));
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, -20.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_four_asymmetric_strict_convex_corner_unions_end_to_end() {
    let mut south_east = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0);
    south_east.rotate_left(2);
    let mirror_segments = |segments: &[ProfileSegment], flip_x: bool, flip_y: bool| {
        segments
            .iter()
            .map(|segment| {
                let mirror = |[x, y]: [f64; 2]| {
                    [
                        if flip_x { 100.0 - x } else { x },
                        if flip_y { 60.0 - y } else { y },
                    ]
                };
                match segment {
                    ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                        start_mm: mirror(*start_mm),
                        end_mm: mirror(*end_mm),
                    },
                    ProfileSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => ProfileSegment::CircularArc {
                        start_mm: mirror(*start_mm),
                        end_mm: mirror(*end_mm),
                        center_mm: mirror(*center_mm),
                        clockwise: if flip_x ^ flip_y {
                            !*clockwise
                        } else {
                            *clockwise
                        },
                    },
                }
            })
            .collect::<Vec<_>>()
    };
    let cases = [
        (
            "south-east",
            south_east.clone(),
            [30.0, 0.0, 100.0, 25.0],
            [[0.0, -5.0, 0.0], [105.0, 60.0, 18.0]],
            0_usize,
        ),
        (
            "south-west",
            mirror_segments(&south_east, true, false),
            [0.0, 0.0, 70.0, 25.0],
            [[-5.0, -5.0, 0.0], [100.0, 60.0, 18.0]],
            1,
        ),
        (
            "north-east",
            mirror_segments(&south_east, false, true),
            [30.0, 35.0, 100.0, 60.0],
            [[0.0, 0.0, 0.0], [105.0, 65.0, 18.0]],
            2,
        ),
        (
            "north-west",
            mirror_segments(&south_east, true, true),
            [0.0, 35.0, 70.0, 60.0],
            [[-5.0, 0.0, 0.0], [100.0, 65.0, 18.0]],
            3,
        ),
    ];
    let radius = 15.0_f64;
    let start_angle = -std::f64::consts::FRAC_PI_2;
    let east_angle = (2.0_f64 / 3.0).acos();
    let east_contact = [100.0, 10.0 + 5.0 * 5.0_f64.sqrt()];
    let south_contact = [110.0 / 3.0, 0.0];
    let cross = |left: [f64; 2], right: [f64; 2]| left[0] * right[1] - left[1] * right[0];
    let arc_integral = radius * 90.0 * (east_angle.sin() - start_angle.sin())
        - radius * 10.0 * (east_angle.cos() - start_angle.cos())
        + radius * radius * (east_angle - start_angle);
    let outside_area = 0.5
        * (cross([40.0, -5.0], [90.0, -5.0])
            + arc_integral
            + cross(east_contact, [100.0, 0.0])
            + cross([100.0, 0.0], south_contact)
            + cross(south_contact, [40.0, -5.0]))
        .abs();
    let profile_area = 1_650.0 + 112.5 * std::f64::consts::PI;
    let overlap_area = profile_area - outside_area;
    let expected_volume = (6_000.0 + profile_area - overlap_area) * 18.0;
    let expected_topology = [14, 21, 9, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (name, segments, expected_overlap_bounds, expected_bounds, classifier_index) in cases {
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments.clone());
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let classifications = [
            profile.strict_convex_line_arc_clipped_south_east_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_south_west_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_north_east_corner_overlap(100.0, 60.0),
            profile.strict_convex_line_arc_clipped_north_west_corner_overlap(100.0, 60.0),
        ];
        assert_eq!(
            classifications
                .iter()
                .filter(|classification| classification.is_some())
                .count(),
            1
        );
        let (classified_area, classified_bounds) = classifications[classifier_index].unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, expected_overlap_bounds);

        let union = backend
            .fuse_mixed_profile(&base.body, &planar_segments(segments), 0.0, 18.0)
            .unwrap();
        assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                union.body.topology.vertex_count,
                union.body.topology.edge_count,
                union.body.topology.face_count,
                union.body.topology.shell_count,
                union.body.topology.solid_count,
            ],
            expected_topology
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
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory.path().join(format!("m127-{name}-union.step"));
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
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() < 1.0e-2,
            "{name}: imported volume {} != expected {expected_volume}",
            imported.body.topology.volume_mm3
        );
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
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
        let stale_path = directory
            .path()
            .join(format!("stale-m127-{name}-union.step"));
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, -20.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_arc_only_clipped_pocket_end_to_end() {
    let segments = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document = mixed_profile_pocket_document(segments, 18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let distance = 10.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [30.0, 15.0, 100.0, 45.0]))
    );
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_line_arc_clipped_side_overlap(100.0, 60.0),
        None
    );
    let expected_volume = 108_000.0 - overlap_area * 8.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let planar_segments = arc_only_clipped_asymmetric_convex_mixed_planar_segments(0.0, 0.0);
    let mut pocket = backend
        .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut pocket,
        "m117-document",
        "m117-pocket",
        Some(10.0),
        Some(&planar_segments),
    )
    .unwrap();
    assert!((pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            pocket.body.topology.vertex_count,
            pocket.body.topology.edge_count,
            pocket.body.topology.face_count,
            pocket.body.topology.shell_count,
            pocket.body.topology.solid_count,
        ],
        [22, 33, 13, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        pocket.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::PocketFloor))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-arc-only-clipped-pocket.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-arc-only-clipped-pocket.step");
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected = mixed_profile_pocket_document(rejected, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_arc_clipped_through_cut_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        None
    );
    let outside_area =
        25.0 + 2.5 * 75.0_f64.sqrt() + 50.0 * (std::f64::consts::FRAC_PI_3 - 3.0_f64.sqrt() / 2.0);
    let overlap_area = 1_300.0 + 25.0 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_side_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [55.0, 15.0, 100.0, 45.0]);
    let expected_volume = 108_000.0 - overlap_area * 18.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let cut = backend
        .cut_mixed_profile(
            &base.body,
            &asymmetric_convex_mixed_planar_segments(30.0, 0.0),
            -1.0,
            20.0,
        )
        .unwrap();
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [20, 30, 12, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-arc-clipped-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-arc-clipped-cut.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(25.0, 0.0),
        asymmetric_convex_mixed_segments(35.0, 0.0),
        asymmetric_convex_mixed_segments(50.0, 0.0),
        asymmetric_convex_mixed_segments(30.0, -20.0),
        asymmetric_convex_mixed_segments(76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_arc_clipped_pocket_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(30.0, 0.0);
    let mut document = mixed_profile_pocket_document(segments, 18.0, 8.0);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let outside_area =
        25.0 + 2.5 * 75.0_f64.sqrt() + 50.0 * (std::f64::consts::FRAC_PI_3 - 3.0_f64.sqrt() / 2.0);
    let overlap_area = 1_300.0 + 25.0 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_line_arc_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [55.0, 15.0, 100.0, 45.0]))
    );
    let expected_volume = 108_000.0 - overlap_area * 8.0;

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let planar_segments = asymmetric_convex_mixed_planar_segments(30.0, 0.0);
    let mut pocket = backend
        .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
        .unwrap();
    capture_polygon_through_cut_references(
        &mut pocket,
        "m116-document",
        "m116-pocket",
        Some(10.0),
        Some(&planar_segments),
    )
    .unwrap();
    assert!((pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            pocket.body.topology.vertex_count,
            pocket.body.topology.edge_count,
            pocket.body.topology.face_count,
            pocket.body.topology.shell_count,
            pocket.body.topology.solid_count,
        ],
        [20, 30, 12, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        pocket.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    for role in [ExactFaceRole::CutLinear, ExactFaceRole::PocketFloor] {
        let reference = package.reference(role).unwrap();
        assert_eq!(reference.profile_feature_id, CUT_PROFILE);
        assert!(reference.has_valid_lineage());
    }
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| { triangle.face_role == Some(ExactFaceRole::PocketFloor) })
    );
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-arc-clipped-pocket.step");
    let model = [(ExactBodyPackage::from(package), Transform::identity())];
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-arc-clipped-pocket.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(35.0, 0.0),
        asymmetric_convex_mixed_segments(50.0, 0.0),
        asymmetric_convex_mixed_segments(30.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected = mixed_profile_pocket_document(rejected, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_mixed_containing_union_end_to_end() {
    let segments = containing_asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_strict_convex_line_arc_profile());
    assert!(!profile.is_line_arc_d_profile());
    assert!(!profile.is_line_arc_capsule_profile());
    assert!(!profile.is_line_arc_rounded_rectangle_profile());

    let profile_area = 9.0 * (1_300.0 + 25.0 * std::f64::consts::PI);
    let expected_volume = profile_area * 18.0;
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
            &containing_asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [10, 15, 7, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
        [[-10.0, -10.0, 0.0], [140.0, 80.0, 18.0]]
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
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-mixed-containing-union.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-mixed-containing-union.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(0.0, 0.0),
        containing_asymmetric_convex_mixed_segments(10.0, 0.0),
        containing_asymmetric_convex_mixed_segments(60.0, 0.0),
        containing_asymmetric_convex_mixed_segments(160.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
    let error = backend
        .fuse_mixed_profile(&base.body, &concave_mixed_planar_segments(), 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_mixed_intersection_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_strict_convex_line_arc_profile());
    assert!(!profile.is_line_arc_d_profile());
    assert!(!profile.is_line_arc_capsule_profile());
    assert!(!profile.is_line_arc_rounded_rectangle_profile());

    let profile_area = 1_300.0 + 25.0 * std::f64::consts::PI;
    let expected_volume = profile_area * 18.0;
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
            &asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
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

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[25.0, 15.0, 0.0], [75.0, 45.0, 18.0]]);
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

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-mixed-intersect.step");
    let model = [(
        ExactBodyPackage::from(package.clone()),
        Transform::identity(),
    )];
    supervisor
        .export_current_model_step(&snapshot, &model, &step_path)
        .unwrap();
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
        .join("stale-asymmetric-convex-mixed-intersect.step");
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

    let rejected =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments.clone());
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Union
        ))
    );
    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
    let error = backend
        .common_mixed_profile(&base.body, &concave_mixed_planar_segments(), 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_arc_clipped_intersection_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        None
    );
    let outside_area =
        25.0 + 2.5 * 75.0_f64.sqrt() + 50.0 * (std::f64::consts::FRAC_PI_3 - 3.0_f64.sqrt() / 2.0);
    let overlap_area = 1_300.0 + 25.0 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_side_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [55.0, 15.0, 100.0, 45.0]);
    let expected_volume = overlap_area * 18.0;

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
            &asymmetric_convex_mixed_planar_segments(30.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            intersection.body.topology.vertex_count,
            intersection.body.topology.edge_count,
            intersection.body.topology.face_count,
            intersection.body.topology.shell_count,
            intersection.body.topology.solid_count,
        ],
        [12, 18, 8, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[55.0, 15.0, 0.0], [100.0, 45.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= 55.0 - 1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= 15.0 - 1.0e-9
            && vertex.position_mm[1] <= 45.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-arc-clipped-intersect.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [12, 18, 8, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-arc-clipped-intersect.step");
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
    for rejected in [
        asymmetric_convex_mixed_segments(35.0, 0.0),
        asymmetric_convex_mixed_segments(50.0, 0.0),
        asymmetric_convex_mixed_segments(30.0, -20.0),
        asymmetric_convex_mixed_segments(76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_arc_clipped_split_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        None
    );
    let outside_area =
        25.0 + 2.5 * 75.0_f64.sqrt() + 50.0 * (std::f64::consts::FRAC_PI_3 - 3.0_f64.sqrt() / 2.0);
    let overlap_area = 1_300.0 + 25.0 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_line_arc_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [55.0, 15.0, 100.0, 45.0]))
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
        .split_mixed_profile(
            &base.body,
            &asymmetric_convex_mixed_planar_segments(30.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ],
        [20, 32, 15, 2, 2]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 2);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-arc-clipped-split.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [32, 48, 20, 2, 2]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-arc-clipped-split.step");
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
    for rejected in [
        asymmetric_convex_mixed_segments(35.0, 0.0),
        asymmetric_convex_mixed_segments(50.0, 0.0),
        asymmetric_convex_mixed_segments(30.0, -20.0),
        asymmetric_convex_mixed_segments(76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_arc_clipped_union_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        None
    );
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        None
    );
    let outside_area =
        25.0 + 2.5 * 75.0_f64.sqrt() + 50.0 * (std::f64::consts::FRAC_PI_3 - 3.0_f64.sqrt() / 2.0);
    let overlap_area = 1_300.0 + 25.0 * std::f64::consts::PI - outside_area;
    let (classified_area, classified_bounds) = profile
        .strict_convex_line_arc_clipped_side_overlap(100.0, 60.0)
        .unwrap();
    assert!((classified_area - overlap_area).abs() < 1.0e-9);
    assert_eq!(classified_bounds, [55.0, 15.0, 100.0, 45.0]);
    let expected_volume = (6_000.0 + outside_area) * 18.0;

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
            &asymmetric_convex_mixed_planar_segments(30.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [14, 21, 9, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [105.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 105.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-arc-clipped-union.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [14, 21, 9, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-arc-clipped-union.step");
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
    for rejected in [
        asymmetric_convex_mixed_segments(35.0, 0.0),
        asymmetric_convex_mixed_segments(50.0, 0.0),
        asymmetric_convex_mixed_segments(30.0, -20.0),
        asymmetric_convex_mixed_segments(76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_arc_only_clipped_intersection_end_to_end() {
    let segments = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let distance = 10.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let expected_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        Some((expected_area, [30.0, 15.0, 100.0, 45.0]))
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
        .common_mixed_profile(
            &base.body,
            &arc_only_clipped_asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((intersection.body.topology.volume_mm3 - expected_area * 18.0).abs() < 2.0e-3);
    assert_eq!(
        [
            intersection.body.topology.vertex_count,
            intersection.body.topology.edge_count,
            intersection.body.topology.face_count,
            intersection.body.topology.shell_count,
            intersection.body.topology.solid_count,
        ],
        [14, 21, 9, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[30.0, 15.0, 0.0], [100.0, 45.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= 30.0 - 1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= 15.0 - 1.0e-9
            && vertex.position_mm[1] <= 45.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-arc-only-clipped-intersect.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_area * 18.0).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [14, 21, 9, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-arc-only-clipped-intersect.step");
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_arc_only_clipped_union_end_to_end() {
    let segments = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let distance = 10.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [30.0, 15.0, 100.0, 45.0]))
    );
    let expected_volume = (6_000.0 + outside_area) * 18.0;

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
            &arc_only_clipped_asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [12, 18, 8, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [105.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 105.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-arc-only-clipped-union.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [12, 18, 8, 1, 1]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-arc-only-clipped-union.step");
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_arc_only_clipped_split_end_to_end() {
    let segments = arc_only_clipped_asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let radius = 15.0_f64;
    let distance = 10.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = 1_650.0 + 112.5 * std::f64::consts::PI - outside_area;
    assert_eq!(
        profile.strict_convex_arc_only_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [30.0, 15.0, 100.0, 45.0]))
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let split = backend
        .split_mixed_profile(
            &base.body,
            &arc_only_clipped_asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ],
        [22, 35, 16, 2, 2]
    );

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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 2);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-arc-only-clipped-split.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [36, 54, 22, 2, 2]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-arc-only-clipped-split.step");
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

    for rejected in [
        arc_only_clipped_asymmetric_convex_mixed_segments(-5.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(20.0, 0.0),
        arc_only_clipped_asymmetric_convex_mixed_segments(0.0, -20.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_clipped_intersection_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(-30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_strict_convex_line_arc_profile());
    assert!(!profile.is_line_arc_d_profile());
    assert!(!profile.is_line_arc_capsule_profile());
    assert!(!profile.is_line_arc_rounded_rectangle_profile());

    let overlap_area = 1_150.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [0.0, 15.0, 45.0, 45.0]))
    );
    let expected_volume = overlap_area * 18.0;
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
            &asymmetric_convex_mixed_planar_segments(-30.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
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

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        intersection.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[0.0, 15.0, 0.0], [45.0, 45.0, 18.0]]);
    let side = package.reference(ExactFaceRole::ArcSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 45.0 + 1.0e-9
            && vertex.position_mm[1] >= 15.0 - 1.0e-9
            && vertex.position_mm[1] <= 45.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-clipped-intersect.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-clipped-intersect.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        asymmetric_convex_mixed_segments(-30.0, -20.0),
        asymmetric_convex_mixed_segments(-76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Intersect
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_clipped_union_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(-30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let overlap_area = 1_150.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [0.0, 15.0, 45.0, 45.0]))
    );
    let expected_volume = 6_150.0 * 18.0;

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
            &asymmetric_convex_mixed_planar_segments(-30.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            union.body.topology.vertex_count,
            union.body.topology.edge_count,
            union.body.topology.face_count,
            union.body.topology.shell_count,
            union.body.topology.solid_count,
        ],
        [16, 24, 10, 1, 1]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        union.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[-5.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let side = package.reference(ExactFaceRole::LinearSide).unwrap();
    assert_eq!(side.profile_feature_id, CUT_PROFILE);
    assert!(side.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .all(|triangle| triangle.face_role != Some(ExactFaceRole::ArcSide))
    );
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -5.0 - 1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-clipped-union.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-clipped-union.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        asymmetric_convex_mixed_segments(-30.0, -20.0),
        asymmetric_convex_mixed_segments(-76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Union
            ))
        );
    }
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_line_clipped_split_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(-30.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    let overlap_area = 1_150.0 + 25.0 * std::f64::consts::PI;
    assert_eq!(
        profile.strict_convex_line_clipped_side_overlap(100.0, 60.0),
        Some((overlap_area, [0.0, 15.0, 45.0, 45.0]))
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
        .split_mixed_profile(
            &base.body,
            &asymmetric_convex_mixed_planar_segments(-30.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ],
        [18, 29, 14, 2, 2]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert!(package.vertices.iter().all(|vertex| {
        vertex.position_mm[0] >= -1.0e-9
            && vertex.position_mm[0] <= 100.0 + 1.0e-9
            && vertex.position_mm[1] >= -1.0e-9
            && vertex.position_mm[1] <= 60.0 + 1.0e-9
    }));
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 2);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("asymmetric-convex-line-clipped-split.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert!((imported.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [28, 42, 18, 2, 2]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-line-clipped-split.step");
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

    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        asymmetric_convex_mixed_segments(-30.0, -20.0),
        asymmetric_convex_mixed_segments(-76.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
    let error = backend
        .split_mixed_profile(&base.body, &concave_mixed_planar_segments(), 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_asymmetric_strict_convex_mixed_split_end_to_end() {
    let segments = asymmetric_convex_mixed_segments(0.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments.clone());
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    let profile = request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert!(profile.is_strict_convex_line_arc_profile());
    assert!(!profile.is_line_arc_d_profile());
    assert!(!profile.is_line_arc_capsule_profile());
    assert!(!profile.is_line_arc_rounded_rectangle_profile());

    let profile_area = 1_300.0 + 25.0 * std::f64::consts::PI;
    let inner_volume = profile_area * 18.0;
    let outer_volume = 108_000.0 - inner_volume;
    assert!(inner_volume > 0.0 && outer_volume > 0.0);
    assert!((inner_volume + outer_volume - 108_000.0).abs() < f64::EPSILON);
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let split = backend
        .split_mixed_profile(
            &base.body,
            &asymmetric_convex_mixed_planar_segments(0.0, 0.0),
            0.0,
            18.0,
        )
        .unwrap();
    assert!((split.body.topology.volume_mm3 - 108_000.0).abs() < 2.0e-3);
    assert_eq!(
        [
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count,
        ],
        [18, 27, 13, 2, 2]
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
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
    assert_closed_manifold(&package);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("asymmetric-convex-mixed-split.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
    assert_eq!(
        [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ],
        [28, 42, 18, 2, 2]
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
    let stale_path = directory
        .path()
        .join("stale-asymmetric-convex-mixed-split.step");
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

    let rejected_union =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected_union.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Union
        ))
    );
    for rejected in [
        asymmetric_convex_mixed_segments(-25.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, rejected);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(
                BooleanOperation::Split
            ))
        );
    }
    let error = backend
        .split_mixed_profile(&base.body, &concave_mixed_planar_segments(), 0.0, 18.0)
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

    let error = backend
        .cut_mixed_profile(&base.body, &concave_mixed_planar_segments(), -1.0, 20.0)
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
fn scheduler_evaluates_chord_side_overlapping_rounded_rectangle_through_cut() {
    let radius = 8.0;
    let corner_deficit = 2.0 * radius * radius * (1.0 - std::f64::consts::PI / 4.0);
    let cases = [
        (40.0, 0.0, 40.0 * 40.0 - corner_deficit),
        (-40.0, 0.0, 40.0 * 40.0 - corner_deficit),
        (0.0, 30.0, 60.0 * 20.0 - corner_deficit),
        (0.0, -30.0, 60.0 * 20.0 - corner_deficit),
    ];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_overlap_area)) in cases.into_iter().enumerate() {
        let segments = translated_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, clipped_bounds) = profile
            .rounded_rectangle_chord_side_overlap(100.0, 60.0)
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert!(clipped_bounds[0] >= 0.0);
        assert!(clipped_bounds[1] >= 0.0);
        assert!(clipped_bounds[2] <= 100.0);
        assert!(clipped_bounds[3] <= 60.0);
        assert_eq!(
            profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
            None
        );
        assert_eq!(
            profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
            None
        );

        let mut direct = backend
            .cut_mixed_profile(&base.body, &planar_segments, -1.0, 20.0)
            .unwrap();
        let expected_volume = (6_000.0 - expected_overlap_area) * 18.0;
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [20, 30, 12, 1, 1]
        );
        capture_polygon_through_cut_references(
            &mut direct,
            "m151-document",
            "m151-cut",
            None,
            Some(&planar_segments),
        )
        .unwrap();

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
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
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("chord-side-rounded-rectangle-cut-{index}.step"));
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
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
        let stale_path = directory.path().join(format!(
            "stale-chord-side-rounded-rectangle-cut-{index}.step"
        ));
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

    for (index, rejected_segments) in [
        translated_rounded_rectangle_segments(28.0, 0.0),
        translated_rounded_rectangle_segments(24.0, 0.0),
        translated_rounded_rectangle_segments(90.0, 0.0),
        concave_mixed_segments(),
    ]
    .into_iter()
    .enumerate()
    {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected_segments);
        let actual = ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION);
        assert_eq!(
            actual,
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut)),
            "rejected chord-side case {index}"
        );
    }

    let existing_corner = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Cut,
        translated_rounded_rectangle_segments(40.0, 30.0),
    );
    let existing_corner_request =
        ExactFeatureChainRequest::from_snapshot(&existing_corner.current(), DEFINITION).unwrap();
    let existing_corner_profile = existing_corner_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        existing_corner_profile.rounded_rectangle_chord_side_overlap(100.0, 60.0),
        None
    );
    assert!(
        existing_corner_profile
            .rounded_rectangle_corner_overlap_area(100.0, 60.0)
            .is_some()
    );

    let pocket =
        mixed_profile_pocket_document(translated_rounded_rectangle_segments(40.0, 0.0), 18.0, 8.0);
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).unwrap();
    assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
    let rejected = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Split,
        translated_rounded_rectangle_segments(28.0, 0.0),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Split
        ))
    );
}

#[test]
fn scheduler_evaluates_chord_side_overlapping_rounded_rectangle_pocket_and_step_round_trip() {
    let radius = 8.0;
    let corner_deficit = 2.0 * radius * radius * (1.0 - std::f64::consts::PI / 4.0);
    let cases = [
        (40.0, 0.0, 40.0 * 40.0 - corner_deficit),
        (-40.0, 0.0, 40.0 * 40.0 - corner_deficit),
        (0.0, 30.0, 60.0 * 20.0 - corner_deficit),
        (0.0, -30.0, 60.0 * 20.0 - corner_deficit),
    ];
    let expected_topology = [20, 30, 12, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_overlap_area)) in cases.into_iter().enumerate() {
        let segments = translated_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_rounded_rectangle_planar_segments(dx, dy);
        let mut document = mixed_profile_pocket_document(segments, 18.0, 8.0);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, clipped_bounds) = profile
            .rounded_rectangle_chord_side_overlap(100.0, 60.0)
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert!(clipped_bounds[0] >= 0.0);
        assert!(clipped_bounds[1] >= 0.0);
        assert!(clipped_bounds[2] <= 100.0);
        assert!(clipped_bounds[3] <= 60.0);

        let mut pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        capture_polygon_through_cut_references(
            &mut pocket,
            "m152-document",
            "m152-pocket",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        let expected_volume = 108_000.0 - expected_overlap_area * 8.0;
        assert_eq!(
            [
                pocket.body.topology.vertex_count,
                pocket.body.topology.edge_count,
                pocket.body.topology.face_count,
                pocket.body.topology.shell_count,
                pocket.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!((pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            pocket.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
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
        assert_eq!(mesh_component_count(&package), 1);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("chord-side-rounded-rectangle-pocket-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-chord-side-rounded-rectangle-pocket-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0003,
            "chord-side pocket STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_rounded_rectangle_segments(28.0, 0.0),
        translated_rounded_rectangle_segments(24.0, 0.0),
        translated_rounded_rectangle_segments(90.0, 0.0),
        concave_mixed_segments(),
    ] {
        let rejected = mixed_profile_pocket_document(rejected_segments, 18.0, 8.0);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_chord_side_overlapping_rounded_rectangle_union_and_step_round_trip() {
    let radius = 8.0;
    let corner_deficit = radius * radius * (1.0 - std::f64::consts::PI / 4.0);
    let profile_area = 60.0 * 40.0 - 4.0 * corner_deficit;
    let cases = [
        (40.0, 0.0, 40.0 * 40.0 - 2.0 * corner_deficit),
        (-40.0, 0.0, 40.0 * 40.0 - 2.0 * corner_deficit),
        (0.0, 30.0, 60.0 * 20.0 - 2.0 * corner_deficit),
        (0.0, -30.0, 60.0 * 20.0 - 2.0 * corner_deficit),
    ];
    let expected_topology = [20, 30, 12, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_overlap_area)) in cases.into_iter().enumerate() {
        let segments = translated_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Union, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        let expected_bounds = [
            [f64::min(0.0, 20.0 + dx), f64::min(0.0, 10.0 + dy), 0.0],
            [f64::max(100.0, 80.0 + dx), f64::max(60.0, 50.0 + dy), 18.0],
        ];
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, clipped_bounds) = profile
            .rounded_rectangle_chord_side_overlap(100.0, 60.0)
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert!(clipped_bounds[0] >= 0.0);
        assert!(clipped_bounds[1] >= 0.0);
        assert!(clipped_bounds[2] <= 100.0);
        assert!(clipped_bounds[3] <= 60.0);
        assert_eq!(
            profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
            None
        );
        assert_eq!(
            profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
            None
        );

        let union = backend
            .fuse_mixed_profile(&base.body, &planar_segments, 0.0, 18.0)
            .unwrap();
        let expected_volume = (6_000.0 + profile_area - expected_overlap_area) * 18.0;
        assert!((union.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                union.body.topology.vertex_count,
                union.body.topology.edge_count,
                union.body.topology.face_count,
                union.body.topology.shell_count,
                union.body.topology.solid_count,
            ],
            expected_topology
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("chord-side-rounded-rectangle-union-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-chord-side-rounded-rectangle-union-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0003,
            "chord-side union STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_rounded_rectangle_segments(28.0, 0.0),
        translated_rounded_rectangle_segments(24.0, 0.0),
        translated_rounded_rectangle_segments(90.0, 0.0),
        concave_mixed_segments(),
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

    let existing_corner = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Union,
        translated_rounded_rectangle_segments(40.0, 30.0),
    );
    let existing_corner_request =
        ExactFeatureChainRequest::from_snapshot(&existing_corner.current(), DEFINITION).unwrap();
    let existing_corner_profile = existing_corner_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        existing_corner_profile.rounded_rectangle_chord_side_overlap(100.0, 60.0),
        None
    );
    assert!(
        existing_corner_profile
            .rounded_rectangle_corner_overlap_area(100.0, 60.0)
            .is_some()
    );
    let rejected = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Split,
        translated_rounded_rectangle_segments(28.0, 0.0),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Split
        ))
    );
}

#[test]
fn scheduler_evaluates_chord_side_overlapping_rounded_rectangle_intersection_and_step_round_trip() {
    let radius = 8.0;
    let corner_deficit = radius * radius * (1.0 - std::f64::consts::PI / 4.0);
    let cases = [
        (
            40.0,
            0.0,
            40.0 * 40.0 - 2.0 * corner_deficit,
            [[60.0, 10.0, 0.0], [100.0, 50.0, 18.0]],
        ),
        (
            -40.0,
            0.0,
            40.0 * 40.0 - 2.0 * corner_deficit,
            [[0.0, 10.0, 0.0], [40.0, 50.0, 18.0]],
        ),
        (
            0.0,
            30.0,
            60.0 * 20.0 - 2.0 * corner_deficit,
            [[20.0, 40.0, 0.0], [80.0, 60.0, 18.0]],
        ),
        (
            0.0,
            -30.0,
            60.0 * 20.0 - 2.0 * corner_deficit,
            [[20.0, 0.0, 0.0], [80.0, 20.0, 18.0]],
        ),
    ];
    let expected_topology = [12, 18, 8, 1, 1];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_overlap_area, expected_bounds)) in cases.into_iter().enumerate() {
        let segments = translated_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Intersect, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        let (overlap_area, clipped_bounds) = profile
            .rounded_rectangle_chord_side_overlap(100.0, 60.0)
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert_eq!(
            clipped_bounds,
            [
                expected_bounds[0][0],
                expected_bounds[0][1],
                expected_bounds[1][0],
                expected_bounds[1][1],
            ]
        );

        let intersection = backend
            .common_mixed_profile(&base.body, &planar_segments, 0.0, 18.0)
            .unwrap();
        let expected_volume = expected_overlap_area * 18.0;
        assert!((intersection.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                intersection.body.topology.vertex_count,
                intersection.body.topology.edge_count,
                intersection.body.topology.face_count,
                intersection.body.topology.shell_count,
                intersection.body.topology.solid_count,
            ],
            expected_topology
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
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory.path().join(format!(
            "chord-side-rounded-rectangle-intersection-{index}.step"
        ));
        let stale_path = directory.path().join(format!(
            "stale-chord-side-rounded-rectangle-intersection-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        let imported_volume_error = (imported.body.topology.volume_mm3 - expected_volume).abs();
        assert!(
            imported_volume_error / expected_volume < 0.0003,
            "chord-side intersection STEP relative volume error {}; actual={}, expected={expected_volume}",
            imported_volume_error / expected_volume,
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

    for rejected_segments in [
        translated_rounded_rectangle_segments(28.0, 0.0),
        translated_rounded_rectangle_segments(24.0, 0.0),
        translated_rounded_rectangle_segments(90.0, 0.0),
        concave_mixed_segments(),
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

    let diagonal = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Intersect,
        translated_rounded_rectangle_segments(40.0, 30.0),
    );
    let diagonal_request =
        ExactFeatureChainRequest::from_snapshot(&diagonal.current(), DEFINITION).unwrap();
    let diagonal_profile = diagonal_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        diagonal_profile.rounded_rectangle_chord_side_overlap(100.0, 60.0),
        None
    );
    assert!(
        diagonal_profile
            .rounded_rectangle_corner_overlap_area(100.0, 60.0)
            .is_some()
    );

    let split = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Split,
        translated_rounded_rectangle_segments(28.0, 0.0),
    );
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Split
        ))
    );
}

#[test]
fn scheduler_evaluates_chord_side_overlapping_rounded_rectangle_split_and_step_round_trip() {
    let radius = 8.0;
    let corner_deficit = 2.0 * radius * radius * (1.0 - std::f64::consts::PI / 4.0);
    let cases = [
        (40.0, 0.0, 40.0 * 40.0 - corner_deficit),
        (-40.0, 0.0, 40.0 * 40.0 - corner_deficit),
        (0.0, 30.0, 60.0 * 20.0 - corner_deficit),
        (0.0, -30.0, 60.0 * 20.0 - corner_deficit),
    ];
    let expected_direct_topology = [20, 32, 15, 2, 2];
    let expected_step_topology = [32, 48, 20, 2, 2];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_overlap_area)) in cases.into_iter().enumerate() {
        let segments = translated_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Split, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let (overlap_area, clipped_bounds) = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .and_then(|profile| profile.rounded_rectangle_chord_side_overlap(100.0, 60.0))
            .unwrap();
        assert!((overlap_area - expected_overlap_area).abs() < 1.0e-9);
        assert!(clipped_bounds[0] >= 0.0);
        assert!(clipped_bounds[1] >= 0.0);
        assert!(clipped_bounds[2] <= 100.0);
        assert!(clipped_bounds[3] <= 60.0);

        let split = backend
            .split_mixed_profile(&base.body, &planar_segments, 0.0, 18.0)
            .unwrap();
        assert_eq!(
            [
                split.body.topology.vertex_count,
                split.body.topology.edge_count,
                split.body.topology.face_count,
                split.body.topology.shell_count,
                split.body.topology.solid_count,
            ],
            expected_direct_topology
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
        assert_eq!(package.vertices, repeated.vertices);
        assert_eq!(package.triangles, repeated.triangles);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        assert_eq!(package.vertices.len(), 152);
        assert_eq!(package.triangles.len(), 296);
        let side = package.reference(ExactFaceRole::ArcSide).unwrap();
        assert_eq!(side.profile_feature_id, CUT_PROFILE);
        assert!(side.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::ArcSide))
        );
        assert_eq!(mesh_component_count(&package), 2);
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);

        let step_path = directory
            .path()
            .join(format!("chord-side-rounded-rectangle-split-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-chord-side-rounded-rectangle-split-{index}.step"
        ));
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
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology
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
        translated_rounded_rectangle_segments(28.0, 0.0),
        translated_rounded_rectangle_segments(24.0, 0.0),
        translated_rounded_rectangle_segments(90.0, 0.0),
        concave_mixed_segments(),
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

    let existing_corner = capsule_boolean_document_with_segments(
        18.0,
        BooleanOperation::Split,
        translated_rounded_rectangle_segments(40.0, 30.0),
    );
    let existing_corner_request =
        ExactFeatureChainRequest::from_snapshot(&existing_corner.current(), DEFINITION).unwrap();
    let existing_corner_profile = existing_corner_request
        .boolean
        .as_ref()
        .and_then(|boolean| boolean.profile.as_ref())
        .unwrap();
    assert_eq!(
        existing_corner_profile.rounded_rectangle_chord_side_overlap(100.0, 60.0),
        None
    );
    assert!(
        existing_corner_profile
            .rounded_rectangle_corner_overlap_area(100.0, 60.0)
            .is_some()
    );
}

#[test]
fn scheduler_evaluates_side_overlapping_rounded_rectangle_cut_and_pocket_step_round_trip() {
    let segments = translated_containing_rounded_rectangle_segments(-80.0, 0.0);
    let mut document =
        capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
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
    assert_eq!(
        request.expected_bounds_mm(),
        [[30.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
    );

    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let mut cut = backend
        .cut_mixed_profile(
            &base.body,
            &translated_containing_rounded_rectangle_planar_segments(-80.0, 0.0),
            -1.0,
            20.0,
        )
        .unwrap();
    let expected_volume = (6_000.0 - 1_800.0) * 18.0;
    assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
    assert_eq!(
        [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );
    let direct_references = capture_polygon_through_cut_references(
        &mut cut,
        "m128-document",
        "m128-cut",
        None,
        Some(&translated_containing_rounded_rectangle_planar_segments(
            -80.0, 0.0,
        )),
    )
    .unwrap();
    assert!(
        direct_references
            .iter()
            .any(|reference| reference.semantic_role == ExactFaceRole::East.semantic_role()),
        "unexpected retained side: {:?}",
        direct_references
            .iter()
            .map(|reference| reference.semantic_role.as_str())
            .collect::<Vec<_>>()
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = supervisor.evaluate_rectangle(&request).unwrap();
    let repeated = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(
        package.identity.result_fingerprint,
        cut.body.result_fingerprint
    );
    assert_eq!(package.identity, repeated.identity);
    assert_eq!(package.references, repeated.references);
    assert_eq!(package.bounds_mm, [[30.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    let wall = package.reference(ExactFaceRole::CutLinear).unwrap();
    assert_eq!(wall.profile_feature_id, CUT_PROFILE);
    assert!(wall.has_valid_lineage());
    assert!(
        package
            .triangles
            .iter()
            .any(|triangle| { triangle.face_role == Some(ExactFaceRole::CutLinear) })
    );
    assert_eq!(package.vertices.len(), 8);
    assert_eq!(package.triangles.len(), 12);
    assert_closed_manifold(&package);
    assert_eq!(mesh_component_count(&package), 1);

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory
        .path()
        .join("side-overlap-rounded-rectangle-cut.step");
    let stale_path = directory
        .path()
        .join("stale-side-overlap-rounded-rectangle-cut.step");
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
    let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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

    for (index, (dx, dy, expected_overlap_area, expected_bounds, expected_side_role)) in [
        (
            80.0,
            0.0,
            1_800.0,
            [[0.0, 0.0, 0.0], [70.0, 60.0, 18.0]],
            ExactFaceRole::West,
        ),
        (
            0.0,
            50.0,
            2_000.0,
            [[0.0, 0.0, 0.0], [100.0, 40.0, 18.0]],
            ExactFaceRole::East,
        ),
        (
            0.0,
            -50.0,
            2_000.0,
            [[0.0, 20.0, 0.0], [100.0, 60.0, 18.0]],
            ExactFaceRole::East,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut oriented_document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Cut,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let oriented_snapshot = oriented_document.current();
        let oriented_request =
            ExactFeatureChainRequest::from_snapshot(&oriented_snapshot, DEFINITION).unwrap();
        assert_eq!(oriented_request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
        assert_eq!(oriented_request.expected_bounds_mm(), expected_bounds);

        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut oriented_cut = backend
            .cut_mixed_profile(&base.body, &planar_segments, -1.0, 20.0)
            .unwrap();
        let expected_volume = (6_000.0 - expected_overlap_area) * 18.0;
        assert!((oriented_cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                oriented_cut.body.topology.vertex_count,
                oriented_cut.body.topology.edge_count,
                oriented_cut.body.topology.face_count,
                oriented_cut.body.topology.shell_count,
                oriented_cut.body.topology.solid_count,
            ],
            [8, 12, 6, 1, 1]
        );
        capture_polygon_through_cut_references(
            &mut oriented_cut,
            "m129-document",
            "m129-cut",
            None,
            Some(&planar_segments),
        )
        .unwrap();

        let oriented_package = supervisor
            .evaluate_rectangle(&oriented_request)
            .unwrap_or_else(|error| panic!("cut orientation {index}: {error:?}"));
        let repeated = supervisor.evaluate_rectangle(&oriented_request).unwrap();
        assert!(oriented_package.is_current(&oriented_snapshot));
        assert_eq!(
            oriented_package.identity.result_fingerprint,
            oriented_cut.body.result_fingerprint
        );
        assert_eq!(oriented_package.identity, repeated.identity);
        assert_eq!(oriented_package.references, repeated.references);
        assert_eq!(oriented_package.bounds_mm, expected_bounds);
        let retained_side = oriented_package.reference(expected_side_role).unwrap();
        assert_eq!(retained_side.profile_feature_id, PROFILE);
        assert!(retained_side.has_valid_lineage());
        let wall = oriented_package
            .reference(ExactFaceRole::CutLinear)
            .unwrap();
        assert_eq!(wall.profile_feature_id, CUT_PROFILE);
        assert!(wall.has_valid_lineage());
        assert_eq!(oriented_package.vertices.len(), 8);
        assert_eq!(oriented_package.triangles.len(), 12);
        assert!(
            oriented_package
                .triangles
                .iter()
                .any(|triangle| { triangle.face_role == Some(ExactFaceRole::CutLinear) })
        );
        assert_closed_manifold(&oriented_package);
        assert_eq!(mesh_component_count(&oriented_package), 1);

        let oriented_step_path = directory
            .path()
            .join(format!("side-overlap-rounded-rectangle-cut-{index}.step"));
        let oriented_stale_path = directory.path().join(format!(
            "stale-side-overlap-rounded-rectangle-cut-{index}.step"
        ));
        let oriented_model = [(
            ExactBodyPackage::from(oriented_package),
            Transform::identity(),
        )];
        let before_revision = oriented_snapshot.revision_id();
        let before_digest = oriented_snapshot.canonical_digest();
        let before_undo = oriented_document.visible_undo_steps();
        supervisor
            .export_current_model_step(&oriented_snapshot, &oriented_model, &oriented_step_path)
            .unwrap();
        let imported = backend
            .import_step(oriented_step_path.to_str().unwrap())
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
        assert_eq!(oriented_document.current().revision_id(), before_revision);
        assert_eq!(
            oriented_document.current().canonical_digest(),
            before_digest
        );
        assert_eq!(oriented_document.visible_undo_steps(), before_undo);

        oriented_document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::new("24", 24.0).unwrap(),
                },
            ]))
            .unwrap();
        std::fs::write(&oriented_stale_path, b"preserved destination").unwrap();
        assert!(
            supervisor
                .export_current_model_step(
                    &oriented_document.current(),
                    &oriented_model,
                    &oriented_stale_path,
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read(&oriented_stale_path).unwrap(),
            b"preserved destination"
        );
    }

    for (index, (dx, dy, expected_overlap_area)) in [
        (-80.0, 0.0, 1_800.0),
        (80.0, 0.0, 1_800.0),
        (0.0, 50.0, 2_000.0),
        (0.0, -50.0, 2_000.0),
    ]
    .into_iter()
    .enumerate()
    {
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut pocket_document = mixed_profile_pocket_document(segments, 18.0, 8.0);
        let pocket_snapshot = pocket_document.current();
        let pocket_request =
            ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
        assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            pocket_request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            pocket_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_side_overlap_area(100.0, 60.0)),
            Some(expected_overlap_area)
        );
        let expected_volume = 108_000.0 - expected_overlap_area * 8.0;
        let mut direct_pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        capture_polygon_through_cut_references(
            &mut direct_pocket,
            "m133-document",
            "m133-pocket",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert!((direct_pocket.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct_pocket.body.topology.vertex_count,
                direct_pocket.body.topology.edge_count,
                direct_pocket.body.topology.face_count,
                direct_pocket.body.topology.shell_count,
                direct_pocket.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );

        let pocket_package = supervisor
            .evaluate_rectangle(&pocket_request)
            .unwrap_or_else(|error| panic!("pocket orientation {index}: {error:?}"));
        let repeated = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        assert!(pocket_package.is_current(&pocket_snapshot));
        assert_eq!(
            pocket_package.identity.result_fingerprint,
            direct_pocket.body.result_fingerprint
        );
        assert_eq!(pocket_package.identity, repeated.identity);
        assert_eq!(pocket_package.references, repeated.references);
        assert_eq!(
            pocket_package.bounds_mm,
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
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
        assert_consistently_oriented_closed_manifold(&pocket_package);
        assert_eq!(mesh_component_count(&pocket_package), 1);

        let pocket_step_path = directory.path().join(format!(
            "side-overlap-rounded-rectangle-pocket-{index}.step"
        ));
        let pocket_stale_path = directory.path().join(format!(
            "stale-side-overlap-rounded-rectangle-pocket-{index}.step"
        ));
        let pocket_model = [(
            ExactBodyPackage::from(pocket_package),
            Transform::identity(),
        )];
        let before_revision = pocket_snapshot.revision_id();
        let before_digest = pocket_snapshot.canonical_digest();
        let before_undo = pocket_document.visible_undo_steps();
        supervisor
            .export_current_model_step(&pocket_snapshot, &pocket_model, &pocket_step_path)
            .unwrap();
        let imported = backend
            .import_step(pocket_step_path.to_str().unwrap())
            .unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(pocket_document.current().revision_id(), before_revision);
        assert_eq!(pocket_document.current().canonical_digest(), before_digest);
        assert_eq!(pocket_document.visible_undo_steps(), before_undo);

        pocket_document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: EXTRUSION,
                    dimension: Dimension::new("24", 24.0).unwrap(),
                },
            ]))
            .unwrap();
        std::fs::write(&pocket_stale_path, b"preserved destination").unwrap();
        assert!(
            supervisor
                .export_current_model_step(
                    &pocket_document.current(),
                    &pocket_model,
                    &pocket_stale_path,
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read(&pocket_stale_path).unwrap(),
            b"preserved destination"
        );
    }
    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 0.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected_segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_corner_overlapping_rounded_rectangle_through_cut_and_step_round_trip() {
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
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;

    for (index, (dx, dy)) in [(80.0, 50.0), (-80.0, 50.0), (80.0, -50.0), (-80.0, -50.0)]
        .into_iter()
        .enumerate()
    {
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
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

        let mut cut = backend
            .cut_mixed_profile(&base.body, &planar_segments, -1.0, 20.0)
            .unwrap();
        let direct_references = ketchup_exact::capture_polygon_through_cut_references(
            &mut cut,
            "m130-cut",
            "corner-overlap",
            None,
            Some(&planar_segments),
        )
        .unwrap();
        assert_eq!(direct_references.len(), 4);
        let direct_topology = [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [14, 21, 9, 1, 1]);
        assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

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
        assert_eq!(package.references.len(), 4);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [14, 21, 9, 1, 1]
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
    }

    for (index, (dx, dy)) in [(80.0, 50.0), (-80.0, 50.0), (80.0, -50.0), (-80.0, -50.0)]
        .into_iter()
        .enumerate()
    {
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut pocket_document = mixed_profile_pocket_document(segments, 18.0, 8.0);
        let pocket_snapshot = pocket_document.current();
        let pocket_request =
            ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
        assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            pocket_request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            pocket_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| profile.rounded_rectangle_corner_overlap_area(100.0, 60.0)),
            Some(expected_overlap_area)
        );

        let expected_pocket_volume = 108_000.0 - expected_overlap_area * 8.0;
        let mut direct_pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        ketchup_exact::capture_polygon_through_cut_references(
            &mut direct_pocket,
            "m134-pocket",
            "corner-overlap",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert!((direct_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct_pocket.body.topology.vertex_count,
                direct_pocket.body.topology.edge_count,
                direct_pocket.body.topology.face_count,
                direct_pocket.body.topology.shell_count,
                direct_pocket.body.topology.solid_count,
            ],
            [16, 24, 10, 1, 1]
        );

        let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        assert!(pocket_package.is_current(&pocket_snapshot));
        assert_eq!(
            pocket_package.identity.result_fingerprint,
            direct_pocket.body.result_fingerprint
        );
        assert_eq!(pocket_package.identity, repeated.identity);
        assert_eq!(pocket_package.references, repeated.references);
        assert_eq!(
            pocket_package.bounds_mm,
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
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
        assert_consistently_oriented_closed_manifold(&pocket_package);
        assert_eq!(mesh_component_count(&pocket_package), 1);

        let step_path = directory
            .path()
            .join(format!("corner-overlap-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-corner-overlap-pocket-{index}.step"));
        let model = [(
            ExactBodyPackage::from(pocket_package),
            Transform::identity(),
        )];
        let before_revision = pocket_snapshot.revision_id();
        let before_digest = pocket_snapshot.canonical_digest();
        let before_undo = pocket_document.visible_undo_steps();
        supervisor
            .export_current_model_step(&pocket_snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
        assert!((imported.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
        assert_eq!(pocket_document.current().revision_id(), before_revision);
        assert_eq!(pocket_document.current().canonical_digest(), before_digest);
        assert_eq!(pocket_document.visible_undo_steps(), before_undo);

        pocket_document
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
                .export_current_model_step(&pocket_document.current(), &model, &stale_path)
                .is_err()
        );
        assert_eq!(
            std::fs::read(&stale_path).unwrap(),
            b"preserved destination"
        );
    }
    for rejected_segments in [
        translated_containing_rounded_rectangle_segments(110.0, 70.0),
        translated_containing_rounded_rectangle_segments(200.0, 100.0),
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected_segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_arc_clipped_corner_rounded_rectangle_through_cut_and_step_round_trip() {
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
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;

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
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
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

        let mut cut = backend
            .cut_mixed_profile(&base.body, &planar_segments, -1.0, 20.0)
            .unwrap();
        let direct_references = ketchup_exact::capture_polygon_through_cut_references(
            &mut cut,
            "m131-cut",
            "arc-clipped-corner-overlap",
            None,
            Some(&planar_segments),
        )
        .unwrap();
        assert_eq!(direct_references.len(), 4);
        let direct_topology = [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1]);
        assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 3.0e-3);

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
        assert_eq!(package.references.len(), 4);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("arc-clipped-corner-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-arc-clipped-corner-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!((imported.body.topology.volume_mm3 - expected_volume).abs() < 3.0e-3);
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
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut pocket_document = mixed_profile_pocket_document(segments, 18.0, 8.0);
        let pocket_snapshot = pocket_document.current();
        let pocket_request =
            ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
        assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            pocket_request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            pocket_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_overlap_area)
        );

        let expected_pocket_volume = 108_000.0 - expected_overlap_area * 8.0;
        let mut direct_pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        ketchup_exact::capture_polygon_through_cut_references(
            &mut direct_pocket,
            "m135-pocket",
            "arc-clipped-corner-overlap",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert!((direct_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 3.0e-3);
        assert_eq!(
            [
                direct_pocket.body.topology.vertex_count,
                direct_pocket.body.topology.edge_count,
                direct_pocket.body.topology.face_count,
                direct_pocket.body.topology.shell_count,
                direct_pocket.body.topology.solid_count,
            ],
            [14, 21, 9, 1, 1]
        );

        let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        assert!(pocket_package.is_current(&pocket_snapshot));
        assert_eq!(
            pocket_package.identity.result_fingerprint,
            direct_pocket.body.result_fingerprint
        );
        assert_eq!(pocket_package.identity, repeated.identity);
        assert_eq!(pocket_package.references, repeated.references);
        assert_eq!(
            pocket_package.bounds_mm,
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
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
        assert_consistently_oriented_closed_manifold(&pocket_package);
        assert_eq!(mesh_component_count(&pocket_package), 1);

        let step_path = directory
            .path()
            .join(format!("arc-clipped-corner-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-arc-clipped-corner-pocket-{index}.step"));
        let model = [(
            ExactBodyPackage::from(pocket_package),
            Transform::identity(),
        )];
        let before_revision = pocket_snapshot.revision_id();
        let before_digest = pocket_snapshot.canonical_digest();
        let before_undo = pocket_document.visible_undo_steps();
        supervisor
            .export_current_model_step(&pocket_snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [14, 21, 9, 1, 1]
        );
        assert!((imported.body.topology.volume_mm3 - expected_pocket_volume).abs() < 3.0e-3);
        assert_eq!(pocket_document.current().revision_id(), before_revision);
        assert_eq!(pocket_document.current().canonical_digest(), before_digest);
        assert_eq!(pocket_document.visible_undo_steps(), before_undo);

        pocket_document
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
                .export_current_model_step(&pocket_document.current(), &model, &stale_path)
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
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected_segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_two_axis_arc_clipped_corner_rounded_rectangle_through_cut_and_step_round_trip()
 {
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
    let expected_volume = 108_000.0 - expected_overlap_area * 18.0;

    for (index, (dx, dy)) in [
        (105.0, 65.0),
        (-105.0, 65.0),
        (105.0, -65.0),
        (-105.0, -65.0),
    ]
    .into_iter()
    .enumerate()
    {
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut document =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, segments);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);
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

        let mut cut = backend
            .cut_mixed_profile(&base.body, &planar_segments, -1.0, 20.0)
            .unwrap();
        let direct_references = ketchup_exact::capture_polygon_through_cut_references(
            &mut cut,
            "m132-cut",
            "two-axis-arc-clipped-corner-overlap",
            None,
            Some(&planar_segments),
        )
        .unwrap();
        assert_eq!(direct_references.len(), 4);
        let direct_topology = [
            cut.body.topology.vertex_count,
            cut.body.topology.edge_count,
            cut.body.topology.face_count,
            cut.body.topology.shell_count,
            cut.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [10, 15, 7, 1, 1]);
        assert!((cut.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);

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
        assert_eq!(package.references.len(), 4);
        let wall = package.reference(ExactFaceRole::CutArc).unwrap();
        assert_eq!(wall.profile_feature_id, CUT_PROFILE);
        assert!(wall.has_valid_lineage());
        assert!(
            package
                .triangles
                .iter()
                .any(|triangle| triangle.face_role == Some(ExactFaceRole::CutArc))
        );
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("two-axis-arc-clipped-corner-cut-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-two-axis-arc-clipped-corner-cut-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
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
    }

    for (index, (dx, dy)) in [
        (105.0, 65.0),
        (-105.0, 65.0),
        (105.0, -65.0),
        (-105.0, -65.0),
    ]
    .into_iter()
    .enumerate()
    {
        let segments = translated_containing_rounded_rectangle_segments(dx, dy);
        let planar_segments = translated_containing_rounded_rectangle_planar_segments(dx, dy);
        let mut pocket_document = mixed_profile_pocket_document(segments, 18.0, 8.0);
        let pocket_snapshot = pocket_document.current();
        let pocket_request =
            ExactFeatureChainRequest::from_snapshot(&pocket_snapshot, DEFINITION).unwrap();
        assert_eq!(pocket_request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(pocket_request.pocket_depth_bits, Some(8.0_f64.to_bits()));
        assert_eq!(
            pocket_request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        assert_eq!(
            pocket_request
                .boolean
                .as_ref()
                .and_then(|boolean| boolean.profile.as_ref())
                .and_then(|profile| {
                    profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0)
                }),
            Some(expected_overlap_area)
        );

        let expected_pocket_volume = 108_000.0 - expected_overlap_area * 8.0;
        let mut direct_pocket = backend
            .cut_mixed_profile(&base.body, &planar_segments, 10.0, 8.0)
            .unwrap();
        ketchup_exact::capture_polygon_through_cut_references(
            &mut direct_pocket,
            "m136-pocket",
            "two-axis-arc-clipped-corner-overlap",
            Some(10.0),
            Some(&planar_segments),
        )
        .unwrap();
        assert!((direct_pocket.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct_pocket.body.topology.vertex_count,
                direct_pocket.body.topology.edge_count,
                direct_pocket.body.topology.face_count,
                direct_pocket.body.topology.shell_count,
                direct_pocket.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );

        let pocket_package = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&pocket_request).unwrap();
        assert!(pocket_package.is_current(&pocket_snapshot));
        assert_eq!(
            pocket_package.identity.result_fingerprint,
            direct_pocket.body.result_fingerprint
        );
        assert_eq!(pocket_package.identity, repeated.identity);
        assert_eq!(pocket_package.references, repeated.references);
        assert_eq!(
            pocket_package.bounds_mm,
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        for role in [ExactFaceRole::CutArc, ExactFaceRole::PocketFloor] {
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
        assert_consistently_oriented_closed_manifold(&pocket_package);
        assert_eq!(mesh_component_count(&pocket_package), 1);

        let step_path = directory
            .path()
            .join(format!("two-axis-arc-clipped-corner-pocket-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-two-axis-arc-clipped-corner-pocket-{index}.step"
        ));
        let model = [(
            ExactBodyPackage::from(pocket_package),
            Transform::identity(),
        )];
        let before_revision = pocket_snapshot.revision_id();
        let before_digest = pocket_snapshot.canonical_digest();
        let before_undo = pocket_document.visible_undo_steps();
        supervisor
            .export_current_model_step(&pocket_snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        assert!((imported.body.topology.volume_mm3 - expected_pocket_volume).abs() < 2.0e-3);
        assert_eq!(pocket_document.current().revision_id(), before_revision);
        assert_eq!(pocket_document.current().canonical_digest(), before_digest);
        assert_eq!(pocket_document.visible_undo_steps(), before_undo);

        pocket_document
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
                .export_current_model_step(&pocket_document.current(), &model, &stale_path,)
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
        concave_mixed_segments(),
        self_intersecting_mixed_segments(),
    ] {
        let rejected =
            capsule_boolean_document_with_segments(18.0, BooleanOperation::Cut, rejected_segments);
        assert_eq!(
            ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION),
            Err(ExactProductError::UnsupportedBoolean(BooleanOperation::Cut))
        );
    }
}

#[test]
fn scheduler_evaluates_side_overlapping_rounded_rectangle_union_and_step_round_trip() {
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
        let expected_bounds = [
            [f64::min(0.0, dx - 10.0), f64::min(0.0, dy - 10.0), 0.0],
            [f64::max(100.0, dx + 110.0), f64::max(60.0, dy + 70.0), 18.0],
        ];
        let expected_volume =
            (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
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
            Some(expected_overlap_area)
        );
        assert_eq!(
            profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
            None
        );
        let union = backend
            .fuse_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("side-overlap-rounded-rectangle-union-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-side-overlap-rounded-rectangle-union-{index}.step"
        ));
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
    }

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
    let expected_volume =
        (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;

    for (index, (dx, dy)) in [(80.0, 50.0), (-80.0, 50.0), (80.0, -50.0), (-80.0, -50.0)]
        .into_iter()
        .enumerate()
    {
        let expected_bounds = [
            [f64::min(0.0, dx - 10.0), f64::min(0.0, dy - 10.0), 0.0],
            [f64::max(100.0, dx + 110.0), f64::max(60.0, dy + 70.0), 18.0],
        ];
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        assert_eq!(
            profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
            Some(expected_overlap_area)
        );
        assert_eq!(
            profile.rounded_rectangle_side_overlap_area(100.0, 60.0),
            None
        );
        let union = backend
            .fuse_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory.path().join(format!(
            "corner-overlap-rounded-rectangle-union-{index}.step"
        ));
        let stale_path = directory.path().join(format!(
            "stale-corner-overlap-rounded-rectangle-union-{index}.step"
        ));
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
    }

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
    let expected_volume =
        (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;
    for (index, (dx, dy)) in [
        (105.0, 65.0),
        (-105.0, 65.0),
        (105.0, -65.0),
        (-105.0, -65.0),
    ]
    .into_iter()
    .enumerate()
    {
        let expected_bounds = [
            [f64::min(0.0, dx - 10.0), f64::min(0.0, dy - 10.0), 0.0],
            [f64::max(100.0, dx + 110.0), f64::max(60.0, dy + 70.0), 18.0],
        ];
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
        assert_eq!(
            profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0),
            Some(expected_overlap_area)
        );
        assert_eq!(
            profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0),
            None
        );

        let union = backend
            .fuse_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("two-axis-arc-clipped-corner-union-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-two-axis-arc-clipped-corner-union-{index}.step"
        ));
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
    }

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
    let expected_volume =
        (6_000.0 + 9_200.0 + 100.0 * std::f64::consts::PI - expected_overlap_area) * 18.0;

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
        let expected_bounds = [
            [f64::min(0.0, dx - 10.0), f64::min(0.0, dy - 10.0), 0.0],
            [f64::max(100.0, dx + 110.0), f64::max(60.0, dy + 70.0), 18.0],
        ];
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Union,
            translated_containing_rounded_rectangle_segments(dx, dy),
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        let profile = request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .unwrap();
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

        let union = backend
            .fuse_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("arc-clipped-corner-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-arc-clipped-corner-union-{index}.step"));
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
    }

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

    let error = backend
        .fuse_mixed_profile(&base.body, &concave_mixed_planar_segments(), 0.0, 18.0)
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile);
}

#[test]
fn scheduler_evaluates_side_overlapping_rounded_rectangle_intersection_and_step_round_trip() {
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

    for (index, (dx, dy, expected_overlap_area, expected_bounds)) in [
        (80.0, 0.0, 1_800.0, [[70.0, 0.0, 0.0], [100.0, 60.0, 18.0]]),
        (-80.0, 0.0, 1_800.0, [[0.0, 0.0, 0.0], [30.0, 60.0, 18.0]]),
        (0.0, 50.0, 2_000.0, [[0.0, 40.0, 0.0], [100.0, 60.0, 18.0]]),
        (0.0, -50.0, 2_000.0, [[0.0, 0.0, 0.0], [100.0, 20.0, 18.0]]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
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
            Some(expected_overlap_area)
        );
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let intersection = backend
            .common_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
                0.0,
                18.0,
            )
            .unwrap();
        let expected_volume = expected_overlap_area * 18.0;
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory.path().join(format!(
            "side-overlap-rounded-rectangle-intersection-{index}.step"
        ));
        let stale_path = directory.path().join(format!(
            "stale-side-overlap-rounded-rectangle-intersection-{index}.step"
        ));
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
    }

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
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let expected_area = 500.0 + 25.0 * std::f64::consts::PI;

    for (index, (dx, dy, expected_bounds)) in [
        (80.0, 50.0, [[70.0, 40.0, 0.0], [100.0, 60.0, 18.0]]),
        (-80.0, 50.0, [[0.0, 40.0, 0.0], [30.0, 60.0, 18.0]]),
        (80.0, -50.0, [[70.0, 0.0, 0.0], [100.0, 20.0, 18.0]]),
        (-80.0, -50.0, [[0.0, 0.0, 0.0], [30.0, 20.0, 18.0]]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
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
            profile.rounded_rectangle_corner_overlap_area(100.0, 60.0),
            Some(expected_area)
        );
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let intersection = backend
            .common_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory.path().join(format!(
            "corner-overlap-rounded-rectangle-intersection-{index}.step"
        ));
        let stale_path = directory.path().join(format!(
            "stale-corner-overlap-rounded-rectangle-intersection-{index}.step"
        ));
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
    }

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
    for (index, (dx, dy, expected_bounds)) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
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
            profile.rounded_rectangle_two_axis_arc_clipped_corner_overlap_area(100.0, 60.0),
            Some(expected_area)
        );
        assert_eq!(
            profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0),
            None
        );
        assert_eq!(request.expected_bounds_mm(), expected_bounds);

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
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
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
        assert_eq!(package.bounds_mm, expected_bounds);
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0][0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[1][0] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[0][1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[1][1] + 1.0e-9
        }));
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let directory = tempfile::tempdir().unwrap();
        let step_path = directory.path().join(format!(
            "two-axis-arc-clipped-corner-rounded-rectangle-intersection-{index}.step"
        ));
        let stale_path = directory.path().join(format!(
            "stale-two-axis-arc-clipped-corner-rounded-rectangle-intersection-{index}.step"
        ));
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
    }

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
    let radius = 10.0_f64;
    let clip_distance = 5.0_f64;
    let straight_extension = 5.0_f64;
    let chord_half = (radius * radius - clip_distance * clip_distance).sqrt();
    let expected_area = (radius - clip_distance) * straight_extension
        + radius * radius * std::f64::consts::FRAC_PI_4
        - 0.5
            * (clip_distance * (radius * radius - clip_distance * clip_distance).sqrt()
                + radius * radius * (clip_distance / radius).asin());
    let cases = [
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
    ];
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (dx, dy, expected_bounds)) in cases.into_iter().enumerate() {
        let mut document = capsule_boolean_document_with_segments(
            18.0,
            BooleanOperation::Intersect,
            translated_containing_rounded_rectangle_segments(dx, dy),
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
            profile.rounded_rectangle_arc_clipped_corner_overlap_area(100.0, 60.0),
            Some(expected_area)
        );
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let intersection = backend
            .common_mixed_profile(
                &base.body,
                &translated_containing_rounded_rectangle_planar_segments(dx, dy),
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
        assert_eq!(package.bounds_mm, expected_bounds);
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
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);
        let step_path = directory.path().join(format!(
            "arc-clipped-corner-rounded-rectangle-intersection-{index}.step"
        ));
        let stale_path = directory.path().join(format!(
            "stale-arc-clipped-corner-rounded-rectangle-intersection-{index}.step"
        ));
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
    }

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
    let invalid_mixed = concave_mixed_planar_segments();
    let error = backend
        .common_mixed_profile(&base.body, &invalid_mixed, -1.0, 20.0)
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

    let broader_mixed = concave_mixed_planar_segments();
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
fn scheduler_evaluates_strict_one_side_overlapping_circular_through_cut() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = std::f64::consts::PI * radius * radius - outside_area;
    let expected_volume = 108_000.0 - overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, host_role)) in [
        ([5.0, 30.0], ExactFaceRole::East),
        ([95.0, 30.0], ExactFaceRole::West),
        ([50.0, 5.0], ExactFaceRole::East),
        ([50.0, 55.0], ExactFaceRole::East),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_CIRCULAR_CUT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        let (classified_area, classified_bounds) = circle.side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert!(classified_bounds.into_iter().all(|value| value >= 0.0));

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: -1.0,
                    radius_mm: radius,
                    height_mm: 20.0,
                },
                CutMode::ThroughAll,
            )
            .unwrap();
        let direct_references = capture_circular_through_cut_references(
            &mut direct,
            "m166-document",
            "m166-cut",
            base_spec,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == host_role.semantic_role()
                && reference.source_element_id == host_role.source_element_id()
        }));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            host_role,
            ExactFaceRole::CutCircle,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-side-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-side-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3,
            "orientation {index}: imported volume {} != expected {expected_volume}",
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

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([50.0, 30.0], 40.0),
        ([50.0, 30.0], 70.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let split = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [95.0, 30.0],
        10.0,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_center_on_side_circular_through_cut() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let overlap_area = 0.5 * std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 - overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        ([0.0, 30.0], [0.0, 20.0, 10.0, 40.0], ExactFaceRole::East),
        (
            [100.0, 30.0],
            [90.0, 20.0, 100.0, 40.0],
            ExactFaceRole::West,
        ),
        ([50.0, 0.0], [40.0, 0.0, 60.0, 10.0], ExactFaceRole::East),
        ([50.0, 60.0], [40.0, 50.0, 60.0, 60.0], ExactFaceRole::East),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: -1.0,
                    radius_mm: radius,
                    height_mm: 20.0,
                },
                CutMode::ThroughAll,
            )
            .unwrap();
        let direct_references = capture_circular_through_cut_references(
            &mut direct,
            "m181-document",
            "m181-cut",
            base_spec,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == host_role.semantic_role()
                && reference.source_element_id == host_role.source_element_id()
        }));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            host_role,
            ExactFaceRole::CutCircle,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-side-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-center-on-side-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([0.0, 10.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([0.0, 30.0], 40.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_center_on_side_circular_blind_pocket() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let pocket_depth = 7.0_f64;
    let overlap_area = 0.5 * std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 - overlap_area * pocket_depth;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        ([0.0, 30.0], [0.0, 20.0, 10.0, 40.0], ExactFaceRole::East),
        (
            [100.0, 30.0],
            [90.0, 20.0, 100.0, 40.0],
            ExactFaceRole::West,
        ),
        ([50.0, 0.0], [40.0, 0.0, 60.0, 10.0], ExactFaceRole::East),
        ([50.0, 60.0], [40.0, 50.0, 60.0, 60.0], ExactFaceRole::East),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(pocket_depth.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 11.0,
                    radius_mm: radius,
                    height_mm: 8.0,
                },
                CutMode::BlindPlanar,
            )
            .unwrap();
        let direct_references = capture_circular_pocket_references(
            &mut direct,
            "m182-document",
            "m182-pocket",
            base_spec,
            11.0,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");
        for semantic_role in [
            host_role.semantic_role(),
            ExactFaceRole::CutCircle.semantic_role(),
            ExactFaceRole::PocketFloor.semantic_role(),
        ] {
            assert!(
                direct_references
                    .iter()
                    .any(|reference| reference.semantic_role == semantic_role)
            );
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-side-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-center-on-side-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([0.0, 10.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([0.0, 1.0e-5], 10.0),
        ([0.0, 30.0], 40.0),
    ] {
        let rejected = circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_center_on_side_circular_union() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let overlap_area = 0.5 * std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 + overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds)) in [
        ([0.0, 30.0], [[-10.0, 0.0, 0.0], [100.0, 60.0, 18.0]]),
        ([100.0, 30.0], [[0.0, 0.0, 0.0], [110.0, 60.0, 18.0]]),
        ([50.0, 0.0], [[0.0, -10.0, 0.0], [100.0, 60.0, 18.0]]),
        ([50.0, 60.0], [[0.0, 0.0, 0.0], [100.0, 70.0, 18.0]]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        let (classified_area, _) = circle.center_on_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);

        let direct = backend
            .fuse_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m185-document", "m185-union").unwrap();
        assert_eq!(direct_references.len(), 3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-side-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-center-on-side-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([0.0, 10.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([0.0, 30.0], 40.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_center_on_side_circular_intersection() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let radius = 10.0_f64;
    let overlap_area = 0.5 * std::f64::consts::PI * radius * radius;
    let expected_volume = overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds, expected_topology)) in [
        (
            [0.0, 30.0],
            [[0.0, 20.0, 0.0], [10.0, 40.0, 18.0]],
            [6, 9, 5, 1, 1],
        ),
        (
            [100.0, 30.0],
            [[90.0, 20.0, 0.0], [100.0, 40.0, 18.0]],
            [4, 6, 4, 1, 1],
        ),
        (
            [50.0, 0.0],
            [[40.0, 0.0, 0.0], [60.0, 10.0, 18.0]],
            [4, 6, 4, 1, 1],
        ),
        (
            [50.0, 60.0],
            [[40.0, 50.0, 0.0], [60.0, 60.0, 18.0]],
            [4, 6, 4, 1, 1],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            expected_bounds,
            [
                [classified_bounds[0], classified_bounds[1], 0.0],
                [classified_bounds[2], classified_bounds[3], 18.0],
            ]
        );

        let direct = backend
            .common_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m187-document", "m187-intersect")
                .unwrap();
        assert_eq!(direct_references.len(), 3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, expected_topology, "orientation {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-side-intersect-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-circle-center-on-side-intersect-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3,
            "orientation {index}: imported volume {} != expected {expected_volume}",
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

    for (center, radius) in [([0.0, 10.0], 10.0), ([0.0, 30.0], 40.0)] {
        let rejected = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_center_on_corner_circular_intersection() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let radius = 10.0_f64;
    let overlap_area = 0.25 * std::f64::consts::PI * radius * radius;
    let expected_volume = overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds, expected_topology)) in [
        (
            [0.0, 0.0],
            [[0.0, 0.0, 0.0], [10.0, 10.0, 18.0]],
            [6, 9, 5, 1, 1],
        ),
        (
            [100.0, 0.0],
            [[90.0, 0.0, 0.0], [100.0, 10.0, 18.0]],
            [6, 9, 5, 1, 1],
        ),
        (
            [0.0, 60.0],
            [[0.0, 50.0, 0.0], [10.0, 60.0, 18.0]],
            [6, 9, 5, 1, 1],
        ),
        (
            [100.0, 60.0],
            [[90.0, 50.0, 0.0], [100.0, 60.0, 18.0]],
            [6, 9, 5, 1, 1],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            expected_bounds,
            [
                [classified_bounds[0], classified_bounds[1], 0.0],
                [classified_bounds[2], classified_bounds[3], 18.0],
            ]
        );

        let direct = backend
            .common_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m188-document", "m188-intersect")
                .unwrap();
        assert_eq!(direct_references.len(), 3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, expected_topology, "corner {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0][0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[1][0] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[0][1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[1][1] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-corner-intersect-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-circle-center-on-corner-intersect-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3,
            "corner {index}: imported volume {} != expected {expected_volume}",
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

    for (center, radius) in [
        ([0.0, 0.0], 60.0),
        ([0.0, 1.0e-5], 10.0),
        ([-20.0, -20.0], 5.0),
    ] {
        let rejected = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let split = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [0.0, 0.0],
        radius,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_outside_center_circular_through_cut() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let overlap_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let expected_volume = 108_000.0 - overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, host_role)) in [
        ([-5.0, 30.0], ExactFaceRole::East),
        ([105.0, 30.0], ExactFaceRole::West),
        ([50.0, -5.0], ExactFaceRole::East),
        ([50.0, 65.0], ExactFaceRole::East),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        let chord_half = (radius * radius - distance * distance).sqrt();
        let expected_overlap_bounds = if center[0] < 0.0 {
            [
                0.0,
                center[1] - chord_half,
                center[0] + radius,
                center[1] + chord_half,
            ]
        } else if center[0] > 100.0 {
            [
                center[0] - radius,
                center[1] - chord_half,
                100.0,
                center[1] + chord_half,
            ]
        } else if center[1] < 0.0 {
            [
                center[0] - chord_half,
                0.0,
                center[0] + chord_half,
                center[1] + radius,
            ]
        } else {
            [
                center[0] - chord_half,
                center[1] - radius,
                center[0] + chord_half,
                60.0,
            ]
        };
        assert_eq!(classified_bounds, expected_overlap_bounds);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: -1.0,
                    radius_mm: radius,
                    height_mm: 20.0,
                },
                CutMode::ThroughAll,
            )
            .unwrap();
        let direct_references = capture_circular_through_cut_references(
            &mut direct,
            "m176-document",
            "m176-cut",
            base_spec,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == host_role.semantic_role()
                && reference.source_element_id == host_role.source_element_id()
        }));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            host_role,
            ExactFaceRole::CutCircle,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-side-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-side-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([-10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([-5.0, 5.0], 10.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let inside = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 30.0],
        radius,
        BooleanOperation::Cut,
    );
    let inside_request =
        ExactFeatureChainRequest::from_snapshot(&inside.current(), DEFINITION).unwrap();
    assert_eq!(
        inside_request
            .boolean
            .as_ref()
            .unwrap()
            .circle
            .unwrap()
            .outside_side_overlap(100.0, 60.0),
        None
    );
    let outside_center = [-5.0, 30.0];
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, outside_center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        outside_center,
        radius,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
    let intersection = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        outside_center,
        radius,
        BooleanOperation::Intersect,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&intersection.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_outside_center_circular_union() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let overlap_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let circle_area = std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 + (circle_area - overlap_area) * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds)) in [
        ([-5.0, 30.0], [[-15.0, 0.0, 0.0], [100.0, 60.0, 18.0]]),
        ([105.0, 30.0], [[0.0, 0.0, 0.0], [115.0, 60.0, 18.0]]),
        ([50.0, -5.0], [[0.0, -15.0, 0.0], [100.0, 60.0, 18.0]]),
        ([50.0, 65.0], [[0.0, 0.0, 0.0], [100.0, 75.0, 18.0]]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        let chord_half = (radius * radius - distance * distance).sqrt();
        let expected_overlap_bounds = if center[0] < 0.0 {
            [
                0.0,
                center[1] - chord_half,
                center[0] + radius,
                center[1] + chord_half,
            ]
        } else if center[0] > 100.0 {
            [
                center[0] - radius,
                center[1] - chord_half,
                100.0,
                center[1] + chord_half,
            ]
        } else if center[1] < 0.0 {
            [
                center[0] - chord_half,
                0.0,
                center[0] + chord_half,
                center[1] + radius,
            ]
        } else {
            [
                center[0] - chord_half,
                center[1] - radius,
                center[0] + chord_half,
                60.0,
            ]
        };
        assert_eq!(classified_bounds, expected_overlap_bounds);

        let direct = backend
            .fuse_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m177-document", "m177-union").unwrap();
        assert_eq!(direct_references.len(), 3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-side-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-side-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([-10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([-5.0, 5.0], 10.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let outside_center = [-5.0, 30.0];
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, outside_center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        outside_center,
        radius,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
    let intersection = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        outside_center,
        radius,
        BooleanOperation::Intersect,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&intersection.current(), DEFINITION).is_ok());
    let through_cut = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        outside_center,
        radius,
        BooleanOperation::Cut,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&through_cut.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_outside_center_circular_intersection() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let overlap_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let expected_volume = overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_topology)) in [
        ([-5.0, 30.0], [6, 9, 5, 1, 1]),
        ([105.0, 30.0], [4, 6, 4, 1, 1]),
        ([50.0, -5.0], [4, 6, 4, 1, 1]),
        ([50.0, 65.0], [4, 6, 4, 1, 1]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [classified_bounds[0], classified_bounds[1], 0.0],
                [classified_bounds[2], classified_bounds[3], 18.0],
            ]
        );

        let direct = backend
            .common_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m178-document", "m178-intersect")
                .unwrap();
        assert_eq!(direct_references.len(), 3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology,
            "orientation {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= classified_bounds[0] - 1.0e-9
                && vertex.position_mm[0] <= classified_bounds[2] + 1.0e-9
                && vertex.position_mm[1] >= classified_bounds[1] - 1.0e-9
                && vertex.position_mm[1] <= classified_bounds[3] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-side-intersect-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-side-intersect-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([-10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([-5.0, 5.0], 10.0),
    ] {
        let rejected = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let outside_center = [-5.0, 30.0];
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, outside_center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        outside_center,
        radius,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
    for operation in [BooleanOperation::Cut, BooleanOperation::Union] {
        let preserved =
            circular_boolean_document(100.0, 60.0, 18.0, outside_center, radius, operation);
        assert!(ExactFeatureChainRequest::from_snapshot(&preserved.current(), DEFINITION).is_ok());
    }
}

#[test]
fn scheduler_evaluates_strict_outside_corner_circular_intersection() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance_x = 5.0_f64;
    let distance_y = 5.0_f64;
    let limit = (radius * radius - distance_y * distance_y).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let overlap_area = primitive(limit) - primitive(distance_x) - distance_y * (limit - distance_x);
    let overlap_extent = limit - distance_x;
    let expected_volume = overlap_area * 18.0;
    let expected_topology = [6, 9, 5, 1, 1];
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds)) in [
        ([-5.0, -5.0], [0.0, 0.0, overlap_extent, overlap_extent]),
        (
            [105.0, -5.0],
            [100.0 - overlap_extent, 0.0, 100.0, overlap_extent],
        ),
        (
            [-5.0, 65.0],
            [0.0, 60.0 - overlap_extent, overlap_extent, 60.0],
        ),
        (
            [105.0, 65.0],
            [100.0 - overlap_extent, 60.0 - overlap_extent, 100.0, 60.0],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [overlap_bounds[0], overlap_bounds[1], 0.0],
                [overlap_bounds[2], overlap_bounds[3], 18.0],
            ]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);

        let direct = backend
            .common_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m194-document", "m194-intersect")
                .unwrap();
        assert_eq!(direct_references.len(), 3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology,
            "corner {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= overlap_bounds[0] - 1.0e-9
                && vertex.position_mm[0] <= overlap_bounds[2] + 1.0e-9
                && vertex.position_mm[1] >= overlap_bounds[1] - 1.0e-9
                && vertex.position_mm[1] <= overlap_bounds[3] + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-corner-intersect-{index}.step"));
        let stale_path = directory.path().join(format!(
            "stale-circle-outside-corner-intersect-{index}.step"
        ));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([-6.0, -8.0], 10.0),
        ([-8.0, -8.0], 10.0),
        ([-5.0, -5.0], 110.0),
        ([-5.0, -1.0e-7], 10.0),
    ] {
        let rejected = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let center = [-5.0, -5.0];
    for operation in [BooleanOperation::Cut, BooleanOperation::Union] {
        let preserved = circular_boolean_document(100.0, 60.0, 18.0, center, radius, operation);
        assert!(ExactFeatureChainRequest::from_snapshot(&preserved.current(), DEFINITION).is_ok());
    }
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_outside_corner_circular_split() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let radius = 10.0_f64;
    let distance_x = 5.0_f64;
    let distance_y = 5.0_f64;
    let limit = (radius * radius - distance_y * distance_y).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let overlap_area = primitive(limit) - primitive(distance_x) - distance_y * (limit - distance_x);
    let overlap_extent = limit - distance_x;
    let expected_volume = 100.0 * 60.0 * 18.0;
    let expected_direct_topology = [12, 20, 11, 2, 2];
    let expected_step_topology = [16, 24, 12, 2, 2];
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds)) in [
        ([-5.0, -5.0], [0.0, 0.0, overlap_extent, overlap_extent]),
        (
            [105.0, -5.0],
            [100.0 - overlap_extent, 0.0, 100.0, overlap_extent],
        ),
        (
            [-5.0, 65.0],
            [0.0, 60.0 - overlap_extent, overlap_extent, 60.0],
        ),
        (
            [105.0, 65.0],
            [100.0 - overlap_extent, 60.0 - overlap_extent, 100.0, 60.0],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);

        let mut direct = backend
            .split_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circular_split_references(&mut direct, "m195-document", "m195-split").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == "extrusion.side(profile_edge=circle)"
                && reference.expected_type == "cylindrical_face"
        }));
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_direct_topology,
            "corner {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory
            .path()
            .join(format!("circle-outside-corner-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-corner-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology,
            "STEP corner {index}"
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([-6.0, -8.0], 10.0),
        ([-8.0, -8.0], 10.0),
        ([-5.0, -5.0], 110.0),
        ([-5.0, -1.0e-7], 10.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_strict_outside_corner_circular_through_cut() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance_x = 5.0_f64;
    let distance_y = 5.0_f64;
    let limit = (radius * radius - distance_y * distance_y).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let overlap_area = primitive(limit) - primitive(distance_x) - distance_y * (limit - distance_x);
    let overlap_extent = limit - distance_x;
    let expected_volume = 108_000.0 - overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        (
            [-5.0, -5.0],
            [0.0, 0.0, overlap_extent, overlap_extent],
            ExactFaceRole::East,
        ),
        (
            [105.0, -5.0],
            [100.0 - overlap_extent, 0.0, 100.0, overlap_extent],
            ExactFaceRole::West,
        ),
        (
            [-5.0, 65.0],
            [0.0, 60.0 - overlap_extent, overlap_extent, 60.0],
            ExactFaceRole::East,
        ),
        (
            [105.0, 65.0],
            [100.0 - overlap_extent, 60.0 - overlap_extent, 100.0, 60.0],
            ExactFaceRole::West,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_CIRCULAR_CUT_EVALUATOR_V1);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: -1.0,
                    radius_mm: radius,
                    height_mm: 20.0,
                },
                CutMode::ThroughAll,
            )
            .unwrap();
        let direct_references = capture_circular_through_cut_references(
            &mut direct,
            "m191-document",
            "m191-cut",
            base_spec,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [10, 15, 7, 1, 1], "corner {index}");
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == host_role.semantic_role()
                && reference.source_element_id == host_role.source_element_id()
        }));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            host_role,
            ExactFaceRole::CutCircle,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-corner-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-corner-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([-6.0, -8.0], 10.0),
        ([-8.0, -8.0], 10.0),
        ([-5.0, -5.0], 110.0),
        ([-5.0, -1.0e-7], 10.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let center = [-5.0, -5.0];
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_outside_corner_circular_blind_pocket() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance_x = 5.0_f64;
    let distance_y = 5.0_f64;
    let pocket_depth = 7.0_f64;
    let limit = (radius * radius - distance_y * distance_y).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let overlap_area = primitive(limit) - primitive(distance_x) - distance_y * (limit - distance_x);
    let overlap_extent = limit - distance_x;
    let expected_volume = 108_000.0 - overlap_area * pocket_depth;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        (
            [-5.0, -5.0],
            [0.0, 0.0, overlap_extent, overlap_extent],
            ExactFaceRole::East,
        ),
        (
            [105.0, -5.0],
            [100.0 - overlap_extent, 0.0, 100.0, overlap_extent],
            ExactFaceRole::West,
        ),
        (
            [-5.0, 65.0],
            [0.0, 60.0 - overlap_extent, overlap_extent, 60.0],
            ExactFaceRole::East,
        ),
        (
            [105.0, 65.0],
            [100.0 - overlap_extent, 60.0 - overlap_extent, 100.0, 60.0],
            ExactFaceRole::West,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(pocket_depth.to_bits()));
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 11.0,
                    radius_mm: radius,
                    height_mm: 8.0,
                },
                CutMode::BlindPlanar,
            )
            .unwrap();
        let direct_references = capture_circular_pocket_references(
            &mut direct,
            "m192-document",
            "m192-pocket",
            base_spec,
            11.0,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "corner {index}");
        for role in [
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(direct_references.iter().any(|reference| {
                reference.semantic_role == role.semantic_role()
                    && reference.source_element_id == role.source_element_id()
            }));
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.reference(ExactFaceRole::Bottom).is_none());
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-corner-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-corner-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([-6.0, -8.0], 10.0),
        ([-8.0, -8.0], 10.0),
        ([-5.0, -5.0], 110.0),
        ([-5.0, -1.0e-7], 10.0),
    ] {
        match try_circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth) {
            Ok(rejected) => assert!(
                ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err()
            ),
            Err(CanonicalError::InvalidFeatureOwnership(POCKET)) => {}
            Err(error) => panic!("unexpected fail-closed error: {error:?}"),
        }
    }
    let center = [-5.0, -5.0];
    match try_circular_pocket_document(100.0, 60.0, 18.0, center, radius, 18.0) {
        Ok(through_depth) => assert!(
            ExactFeatureChainRequest::from_snapshot(&through_depth.current(), DEFINITION).is_err()
        ),
        Err(CanonicalError::InvalidFeatureOwnership(POCKET)) => {}
        Err(error) => panic!("unexpected through-depth rejection: {error:?}"),
    }
    let through_cut =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
    assert!(ExactFeatureChainRequest::from_snapshot(&through_cut.current(), DEFINITION).is_ok());
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_outside_corner_circular_union() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance_x = 5.0_f64;
    let distance_y = 5.0_f64;
    let limit = (radius * radius - distance_y * distance_y).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let overlap_area = primitive(limit) - primitive(distance_x) - distance_y * (limit - distance_x);
    let overlap_extent = limit - distance_x;
    let circle_area = std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 + (circle_area - overlap_area) * 18.0;
    let expected_topology = [10, 15, 7, 1, 1];
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, expected_bounds)) in [
        (
            [-5.0, -5.0],
            [0.0, 0.0, overlap_extent, overlap_extent],
            [[-15.0, -15.0, 0.0], [100.0, 60.0, 18.0]],
        ),
        (
            [105.0, -5.0],
            [100.0 - overlap_extent, 0.0, 100.0, overlap_extent],
            [[0.0, -15.0, 0.0], [115.0, 60.0, 18.0]],
        ),
        (
            [-5.0, 65.0],
            [0.0, 60.0 - overlap_extent, overlap_extent, 60.0],
            [[-15.0, 0.0, 0.0], [100.0, 75.0, 18.0]],
        ),
        (
            [105.0, 65.0],
            [100.0 - overlap_extent, 60.0 - overlap_extent, 100.0, 60.0],
            [[0.0, 0.0, 0.0], [115.0, 75.0, 18.0]],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);

        let direct = backend
            .fuse_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m193-document", "m193-union").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology,
            "corner {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0][0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[1][0] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[0][1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[1][1] + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-corner-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-corner-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([-6.0, -8.0], 10.0),
        ([-8.0, -8.0], 10.0),
        ([-5.0, -5.0], 110.0),
        ([-5.0, -1.0e-7], 10.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let center = [-5.0, -5.0];
    let through_cut =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
    assert!(ExactFeatureChainRequest::from_snapshot(&through_cut.current(), DEFINITION).is_ok());
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_center_on_corner_circular_through_cut() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let overlap_area = 0.25 * std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 - overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        ([0.0, 0.0], [0.0, 0.0, 10.0, 10.0], ExactFaceRole::East),
        ([100.0, 0.0], [90.0, 0.0, 100.0, 10.0], ExactFaceRole::West),
        ([0.0, 60.0], [0.0, 50.0, 10.0, 60.0], ExactFaceRole::East),
        (
            [100.0, 60.0],
            [90.0, 50.0, 100.0, 60.0],
            ExactFaceRole::West,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: -1.0,
                    radius_mm: radius,
                    height_mm: 20.0,
                },
                CutMode::ThroughAll,
            )
            .unwrap();
        let direct_references = capture_circular_through_cut_references(
            &mut direct,
            "m183-document",
            "m183-cut",
            base_spec,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [10, 15, 7, 1, 1], "corner {index}");
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == host_role.semantic_role()
                && reference.source_element_id == host_role.source_element_id()
        }));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            host_role,
            ExactFaceRole::CutCircle,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-corner-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
        );
    }

    for (center, radius) in [
        ([0.0, 0.0], 60.0),
        ([0.0, 1.0e-5], 10.0),
        ([-20.0, -20.0], 5.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let center = [0.0, 0.0];
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, center, radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
    let union =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
    assert!(ExactFeatureChainRequest::from_snapshot(&union.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_center_on_corner_circular_blind_pocket() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let pocket_depth = 7.0_f64;
    let overlap_area = 0.25 * std::f64::consts::PI * radius * radius;
    let expected_volume = 108_000.0 - overlap_area * pocket_depth;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        ([0.0, 0.0], [0.0, 0.0, 10.0, 10.0], ExactFaceRole::East),
        ([100.0, 0.0], [90.0, 0.0, 100.0, 10.0], ExactFaceRole::West),
        ([0.0, 60.0], [0.0, 50.0, 10.0, 60.0], ExactFaceRole::East),
        (
            [100.0, 60.0],
            [90.0, 50.0, 100.0, 60.0],
            ExactFaceRole::West,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let document = circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(pocket_depth.to_bits()));
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 11.0,
                    radius_mm: radius,
                    height_mm: 8.0,
                },
                CutMode::BlindPlanar,
            )
            .unwrap();
        let direct_references = capture_circular_pocket_references(
            &mut direct,
            "m184-document",
            "m184-pocket",
            base_spec,
            11.0,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "corner {index}");
        for role in [
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(direct_references.iter().any(|reference| {
                reference.semantic_role == role.semantic_role()
                    && reference.source_element_id == role.source_element_id()
            }));
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.reference(ExactFaceRole::Bottom).is_none());
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-corner-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
        );
    }

    for (center, radius) in [
        ([0.0, 0.0], 60.0),
        ([0.0, 1.0e-5], 10.0),
        ([-20.0, -20.0], 5.0),
    ] {
        let rejected = circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let center = [0.0, 0.0];
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
    let union =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
    assert!(ExactFeatureChainRequest::from_snapshot(&union.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_center_on_corner_circular_union() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let overlap_area = 0.25 * std::f64::consts::PI * radius * radius;
    let expected_volume =
        108_000.0 + (std::f64::consts::PI * radius * radius - overlap_area) * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds)) in [
        ([0.0, 0.0], [[-10.0, -10.0, 0.0], [100.0, 60.0, 18.0]]),
        ([100.0, 0.0], [[0.0, -10.0, 0.0], [110.0, 60.0, 18.0]]),
        ([0.0, 60.0], [[-10.0, 0.0, 0.0], [100.0, 70.0, 18.0]]),
        ([100.0, 60.0], [[0.0, 0.0, 0.0], [110.0, 70.0, 18.0]]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            classified_bounds,
            [
                center[0].min(100.0 - radius).max(0.0),
                center[1].min(60.0 - radius).max(0.0),
                center[0].max(radius).min(100.0),
                center[1].max(radius).min(60.0),
            ]
        );

        let direct = backend
            .fuse_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m186-document", "m186-union").unwrap();
        assert_eq!(direct_references.len(), 3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [10, 15, 7, 1, 1], "corner {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0][0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[1][0] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[0][1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[1][1] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-corner-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-center-on-corner-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([0.0, 0.0], 60.0),
        ([0.0, 1.0e-5], 10.0),
        ([-20.0, -20.0], 5.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let center = [0.0, 0.0];
    let split =
        circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_corner_overlapping_circular_through_cut() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let cap = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let limit = (radius * radius - distance * distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let shared_outside =
        primitive(limit) - distance * limit - primitive(distance) + distance * distance;
    let overlap_area = std::f64::consts::PI * radius * radius - 2.0 * cap + shared_outside;
    let expected_volume = 108_000.0 - overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, host_role)) in [
        ([5.0, 5.0], ExactFaceRole::East),
        ([95.0, 5.0], ExactFaceRole::West),
        ([5.0, 55.0], ExactFaceRole::East),
        ([95.0, 55.0], ExactFaceRole::West),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) = circle.corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            classified_bounds,
            [
                0.0_f64.max(center[0] - radius),
                0.0_f64.max(center[1] - radius),
                100.0_f64.min(center[0] + radius),
                60.0_f64.min(center[1] + radius)
            ]
        );
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: -1.0,
                    radius_mm: radius,
                    height_mm: 20.0,
                },
                CutMode::ThroughAll,
            )
            .unwrap();
        let direct_references = capture_circular_through_cut_references(
            &mut direct,
            "m171-document",
            "m171-cut",
            base_spec,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [10, 15, 7, 1, 1]
        );
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == host_role.semantic_role()
                && reference.source_element_id == host_role.source_element_id()
        }));

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            host_role,
            ExactFaceRole::CutCircle,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-corner-cut-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-corner-cut-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
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
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([10.0, 10.0], 10.0),
        ([-20.0, -20.0], 5.0),
        ([8.0, 8.0], 10.0),
        ([50.0, 30.0], 55.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Cut);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, [5.0, 5.0], 10.0, 7.0);
    let pocket_request =
        ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).unwrap();
    assert!(
        pocket_request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.circle)
            .and_then(|circle| circle.corner_overlap(100.0, 60.0))
            .is_some()
    );
    let union =
        circular_boolean_document(100.0, 60.0, 18.0, [5.0, 5.0], 10.0, BooleanOperation::Union);
    assert!(ExactFeatureChainRequest::from_snapshot(&union.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_corner_overlapping_circular_blind_pocket() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let pocket_depth = 7.0_f64;
    let cap = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let limit = (radius * radius - distance * distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let shared_outside =
        primitive(limit) - distance * limit - primitive(distance) + distance * distance;
    let overlap_area = std::f64::consts::PI * radius * radius - 2.0 * cap + shared_outside;
    let expected_volume = 108_000.0 - overlap_area * pocket_depth;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, host_role)) in [
        ([5.0, 5.0], ExactFaceRole::East),
        ([95.0, 5.0], ExactFaceRole::West),
        ([5.0, 55.0], ExactFaceRole::East),
        ([95.0, 55.0], ExactFaceRole::West),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(pocket_depth.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) = circle.corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            classified_bounds,
            [
                0.0_f64.max(center[0] - radius),
                0.0_f64.max(center[1] - radius),
                100.0_f64.min(center[0] + radius),
                60.0_f64.min(center[1] + radius)
            ]
        );
        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 11.0,
                    radius_mm: radius,
                    height_mm: 8.0,
                },
                CutMode::BlindPlanar,
            )
            .unwrap();
        let direct_references = capture_circular_pocket_references(
            &mut direct,
            "m172-document",
            "m172-pocket",
            base_spec,
            11.0,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        for semantic_role in [
            host_role.semantic_role(),
            ExactFaceRole::CutCircle.semantic_role(),
            ExactFaceRole::PocketFloor.semantic_role(),
        ] {
            assert!(
                direct_references
                    .iter()
                    .any(|reference| reference.semantic_role == semantic_role)
            );
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        for role in [
            ExactFaceRole::Top,
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-corner-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-corner-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([10.0, 10.0], 10.0),
        ([-20.0, -20.0], 5.0),
        ([8.0, 8.0], 10.0),
        ([50.0, 30.0], 55.0),
    ] {
        let rejected = circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let one_side = circular_pocket_document(100.0, 60.0, 18.0, [5.0, 30.0], radius, pocket_depth);
    let one_side_request =
        ExactFeatureChainRequest::from_snapshot(&one_side.current(), DEFINITION).unwrap();
    let one_side_circle = one_side_request.boolean.as_ref().unwrap().circle.unwrap();
    assert!(one_side_circle.side_overlap(100.0, 60.0).is_some());
    assert_eq!(one_side_circle.corner_overlap(100.0, 60.0), None);
    for invalid_depth in [18.0, 19.0] {
        let mut rejected =
            circular_pocket_document(100.0, 60.0, 18.0, [5.0, 5.0], radius, pocket_depth);
        let before = rejected.current();
        assert!(
            rejected
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetFeatureDimension {
                        id: POCKET,
                        dimension: Dimension::new(invalid_depth.to_string(), invalid_depth)
                            .unwrap(),
                    },
                ]))
                .is_err()
        );
        assert_eq!(rejected.current().revision_id(), before.revision_id());
        assert_eq!(
            rejected.current().canonical_digest(),
            before.canonical_digest()
        );
    }
    let union = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 5.0],
        radius,
        BooleanOperation::Union,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&union.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_one_side_overlapping_circular_blind_pocket() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let pocket_depth = 7.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = std::f64::consts::PI * radius * radius - outside_area;
    let expected_volume = 108_000.0 - overlap_area * pocket_depth;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, host_role)) in [
        ([5.0, 30.0], ExactFaceRole::East),
        ([95.0, 30.0], ExactFaceRole::West),
        ([50.0, 5.0], ExactFaceRole::East),
        ([50.0, 55.0], ExactFaceRole::East),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(pocket_depth.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        let (classified_area, _) = circle.side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 11.0,
                    radius_mm: radius,
                    height_mm: 8.0,
                },
                CutMode::BlindPlanar,
            )
            .unwrap();
        let direct_references = capture_circular_pocket_references(
            &mut direct,
            "m167-document",
            "m167-pocket",
            base_spec,
            11.0,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            [12, 18, 8, 1, 1]
        );
        for semantic_role in [
            host_role.semantic_role(),
            ExactFaceRole::CutCircle.semantic_role(),
            ExactFaceRole::PocketFloor.semantic_role(),
        ] {
            assert!(
                direct_references
                    .iter()
                    .any(|reference| reference.semantic_role == semantic_role),
                "missing {semantic_role} for center {center:?}; got {:?}",
                direct_references
                    .iter()
                    .map(|reference| reference.semantic_role.as_str())
                    .collect::<Vec<_>>()
            );
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
        for role in [
            ExactFaceRole::Top,
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-side-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-side-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([50.0, 30.0], 40.0),
        ([50.0, 30.0], 70.0),
    ] {
        let rejected = circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    for invalid_depth in [18.0, 19.0] {
        let mut rejected =
            circular_pocket_document(100.0, 60.0, 18.0, [95.0, 30.0], radius, pocket_depth);
        let before = rejected.current();
        assert!(
            rejected
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetFeatureDimension {
                        id: POCKET,
                        dimension: Dimension::new(invalid_depth.to_string(), invalid_depth)
                            .unwrap(),
                    },
                ]))
                .is_err()
        );
        assert_eq!(rejected.current().revision_id(), before.revision_id());
        assert_eq!(
            rejected.current().canonical_digest(),
            before.canonical_digest()
        );
    }
    let split = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [95.0, 30.0],
        radius,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&split.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_one_side_overlapping_circular_union() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let expected_volume = 100.0 * 60.0 * 18.0 + outside_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds)) in [
        ([5.0, 30.0], [[-5.0, 0.0, 0.0], [100.0, 60.0, 18.0]]),
        ([95.0, 30.0], [[0.0, 0.0, 0.0], [105.0, 60.0, 18.0]]),
        ([50.0, 5.0], [[0.0, -5.0, 0.0], [100.0, 60.0, 18.0]]),
        ([50.0, 55.0], [[0.0, 0.0, 0.0], [100.0, 65.0, 18.0]]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        let (overlap_area, _) = circle.side_overlap(100.0, 60.0).unwrap();
        assert!(
            (std::f64::consts::PI * radius * radius - overlap_area - outside_area).abs() < 1.0e-9
        );

        let direct = backend
            .fuse_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m169-document", "m169-union").unwrap();
        assert_eq!(direct_references.len(), 3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0][0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[1][0] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[0][1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[1][1] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-side-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-side-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            direct_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([50.0, 30.0], 40.0),
        ([50.0, 30.0], 55.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_strict_one_side_overlapping_circular_intersection() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let outside_area = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let overlap_area = std::f64::consts::PI * radius * radius - outside_area;
    let expected_volume = overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_topology)) in [
        ([5.0, 30.0], [6, 9, 5, 1, 1]),
        ([95.0, 30.0], [4, 6, 4, 1, 1]),
        ([50.0, 5.0], [6, 9, 5, 1, 1]),
        ([50.0, 55.0], [6, 9, 5, 1, 1]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        let (classified_area, classified_bounds) = circle.side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [classified_bounds[0], classified_bounds[1], 0.0],
                [classified_bounds[2], classified_bounds[3], 18.0],
            ]
        );

        let direct = backend
            .common_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m168-document", "m168-intersect")
                .unwrap();
        assert_eq!(direct_references.len(), 3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= classified_bounds[0] - 1.0e-9
                && vertex.position_mm[0] <= classified_bounds[2] + 1.0e-9
                && vertex.position_mm[1] >= classified_bounds[1] - 1.0e-9
                && vertex.position_mm[1] <= classified_bounds[3] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-side-intersect-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-side-intersect-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3,
            "orientation {index}: imported volume {} != expected {expected_volume}",
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

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([50.0, 30.0], 40.0),
        ([50.0, 30.0], 70.0),
    ] {
        let rejected = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_strict_corner_overlapping_circular_union() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let cap = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let limit = (radius * radius - distance * distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let shared_outside =
        primitive(limit) - distance * limit - primitive(distance) + distance * distance;
    let overlap_area = std::f64::consts::PI * radius * radius - 2.0 * cap + shared_outside;
    let expected_volume =
        100.0 * 60.0 * 18.0 + (std::f64::consts::PI * radius * radius - overlap_area) * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_bounds, expected_topology)) in [
        (
            [5.0, 5.0],
            [[-5.0, -5.0, 0.0], [100.0, 60.0, 18.0]],
            [10, 15, 7, 1, 1],
        ),
        (
            [95.0, 5.0],
            [[0.0, -5.0, 0.0], [105.0, 60.0, 18.0]],
            [10, 15, 7, 1, 1],
        ),
        (
            [5.0, 55.0],
            [[-5.0, 0.0, 0.0], [100.0, 65.0, 18.0]],
            [10, 15, 7, 1, 1],
        ),
        (
            [95.0, 55.0],
            [[0.0, 0.0, 0.0], [105.0, 65.0, 18.0]],
            [10, 15, 7, 1, 1],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
        assert_eq!(request.expected_bounds_mm(), expected_bounds);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        let (classified_area, _) = circle.corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);

        let direct = backend
            .fuse_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circle_extrusion_references(&direct, "m175-document", "m175-union").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology,
            "orientation {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, expected_bounds);
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= expected_bounds[0][0] - 1.0e-9
                && vertex.position_mm[0] <= expected_bounds[1][0] + 1.0e-9
                && vertex.position_mm[1] >= expected_bounds[0][1] - 1.0e-9
                && vertex.position_mm[1] <= expected_bounds[1][1] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-corner-union-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-corner-union-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([10.0, 10.0], 10.0),
        ([-20.0, -20.0], 5.0),
        ([8.0, 8.0], 10.0),
        ([50.0, 30.0], 55.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Union);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let one_side = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 30.0],
        radius,
        BooleanOperation::Union,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&one_side.current(), DEFINITION).is_ok());
    let intersection = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 5.0],
        radius,
        BooleanOperation::Intersect,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&intersection.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_corner_overlapping_circular_intersection() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let cap = radius * radius * (distance / radius).acos()
        - distance * (radius * radius - distance * distance).sqrt();
    let limit = (radius * radius - distance * distance).sqrt();
    let primitive = |value: f64| {
        0.5 * (value * (radius * radius - value * value).sqrt()
            + radius * radius * (value / radius).asin())
    };
    let shared_outside =
        primitive(limit) - distance * limit - primitive(distance) + distance * distance;
    let overlap_area = std::f64::consts::PI * radius * radius - 2.0 * cap + shared_outside;
    let expected_volume = overlap_area * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_topology)) in [
        ([5.0, 5.0], [8, 12, 6, 1, 1]),
        ([95.0, 5.0], [6, 9, 5, 1, 1]),
        ([5.0, 55.0], [8, 12, 6, 1, 1]),
        ([95.0, 55.0], [6, 9, 5, 1, 1]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) = circle.corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [classified_bounds[0], classified_bounds[1], 0.0],
                [classified_bounds[2], classified_bounds[3], 18.0],
            ]
        );

        let direct = backend
            .common_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_topology
        );
        let direct_references =
            capture_circle_extrusion_references(&direct, "m173-document", "m173-intersect")
                .unwrap();
        assert_eq!(direct_references.len(), 3);

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= classified_bounds[0] - 1.0e-9
                && vertex.position_mm[0] <= classified_bounds[2] + 1.0e-9
                && vertex.position_mm[1] >= classified_bounds[1] - 1.0e-9
                && vertex.position_mm[1] <= classified_bounds[3] + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-corner-intersect-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-corner-intersect-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_topology
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([10.0, 10.0], 10.0),
        ([-20.0, -20.0], 5.0),
        ([8.0, 8.0], 10.0),
        ([50.0, 30.0], 55.0),
    ] {
        let rejected = circular_boolean_document(
            100.0,
            60.0,
            18.0,
            center,
            radius,
            BooleanOperation::Intersect,
        );
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let one_side = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 30.0],
        radius,
        BooleanOperation::Intersect,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&one_side.current(), DEFINITION).is_ok());
    let union = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 5.0],
        radius,
        BooleanOperation::Union,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&union.current(), DEFINITION).is_ok());
    let cut =
        circular_boolean_document(100.0, 60.0, 18.0, [5.0, 5.0], radius, BooleanOperation::Cut);
    assert!(ExactFeatureChainRequest::from_snapshot(&cut.current(), DEFINITION).is_ok());
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, [5.0, 5.0], radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_corner_overlapping_circular_split() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let radius = 10.0_f64;
    let expected_volume = 100.0 * 60.0 * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_direct_topology, expected_step_topology)) in [
        ([5.0, 5.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
        ([95.0, 5.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
        ([5.0, 55.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
        ([95.0, 55.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0; 3], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert!(circle.corner_overlap(100.0, 60.0).is_some());

        let mut direct = backend
            .split_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circular_split_references(&mut direct, "m174-document", "m174-split").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == "extrusion.side(profile_edge=circle)"
                && reference.expected_type == "cylindrical_face"
        }));
        assert_eq!(
            [
                direct.body.topology.vertex_count,
                direct.body.topology.edge_count,
                direct.body.topology.face_count,
                direct.body.topology.shell_count,
                direct.body.topology.solid_count,
            ],
            expected_direct_topology,
            "orientation {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory
            .path()
            .join(format!("circle-corner-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-corner-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        assert_eq!(
            [
                imported.body.topology.vertex_count,
                imported.body.topology.edge_count,
                imported.body.topology.face_count,
                imported.body.topology.shell_count,
                imported.body.topology.solid_count,
            ],
            expected_step_topology,
            "STEP orientation {index}"
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([10.0, 10.0], 10.0),
        ([-20.0, -20.0], 5.0),
        ([8.0, 8.0], 10.0),
        ([50.0, 30.0], 55.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let one_side = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 30.0],
        radius,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&one_side.current(), DEFINITION).is_ok());
    let union = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [5.0, 5.0],
        radius,
        BooleanOperation::Union,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&union.current(), DEFINITION).is_ok());
    for operation in [BooleanOperation::Cut, BooleanOperation::Intersect] {
        let accepted = circular_boolean_document(100.0, 60.0, 18.0, [5.0, 5.0], radius, operation);
        assert!(ExactFeatureChainRequest::from_snapshot(&accepted.current(), DEFINITION).is_ok());
    }
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, [5.0, 5.0], radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_strict_one_side_overlapping_circular_split() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let expected_volume = 100.0 * 60.0 * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_direct_topology, expected_step_topology)) in [
        ([5.0, 30.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
        ([95.0, 30.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
        ([50.0, 5.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
        ([50.0, 55.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0; 3], [100.0, 60.0, 18.0]]
        );
        assert!(
            request
                .boolean
                .as_ref()
                .unwrap()
                .circle
                .unwrap()
                .side_overlap(100.0, 60.0)
                .is_some()
        );

        let mut direct = backend
            .split_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circular_split_references(&mut direct, "m170-document", "m170-split").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == "extrusion.side(profile_edge=circle)"
                && reference.expected_type == "cylindrical_face"
        }));
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(
            direct_topology, expected_direct_topology,
            "orientation {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory
            .path()
            .join(format!("circle-side-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-side-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        let imported_topology = [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ];
        assert_eq!(
            imported_topology, expected_step_topology,
            "STEP orientation {index}"
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([50.0, 30.0], 40.0),
        ([50.0, 30.0], 70.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let contained = circular_boolean_document(
        100.0,
        60.0,
        18.0,
        [50.0, 30.0],
        10.0,
        BooleanOperation::Split,
    );
    assert!(ExactFeatureChainRequest::from_snapshot(&contained.current(), DEFINITION).is_ok());
}

#[test]
fn scheduler_evaluates_center_on_side_circular_split() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let radius = 10.0_f64;
    let expected_volume = 100.0 * 60.0 * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_direct_topology, expected_step_topology)) in [
        ([0.0, 30.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
        ([100.0, 30.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
        ([50.0, 0.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
        ([50.0, 60.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0; 3], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        let (classified_area, _) = circle.center_on_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - 0.5 * std::f64::consts::PI * radius * radius).abs() < 1.0e-9);

        let mut direct = backend
            .split_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circular_split_references(&mut direct, "m189-document", "m189-split").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == "extrusion.side(profile_edge=circle)"
                && reference.expected_type == "cylindrical_face"
        }));
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(
            direct_topology, expected_direct_topology,
            "orientation {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-side-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-center-on-side-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        let imported_topology = [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ];
        assert_eq!(
            imported_topology, expected_step_topology,
            "STEP orientation {index}"
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([0.0, 10.0], 10.0),
        ([0.0, 30.0], 30.0),
        ([-20.0, 30.0], 5.0),
        ([50.0, 30.0], 70.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_center_on_corner_circular_split() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let radius = 10.0_f64;
    let expected_volume = 100.0 * 60.0 * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, center) in [[0.0, 0.0], [100.0, 0.0], [0.0, 60.0], [100.0, 60.0]]
        .into_iter()
        .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0; 3], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert_eq!(circle.outside_side_overlap(100.0, 60.0), None);
        assert_eq!(circle.center_on_side_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.center_on_corner_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - 0.25 * std::f64::consts::PI * radius * radius).abs() < 1.0e-9);
        assert_eq!(
            classified_bounds,
            [
                (center[0] - radius).max(0.0),
                (center[1] - radius).max(0.0),
                (center[0] + radius).min(100.0),
                (center[1] + radius).min(60.0),
            ]
        );

        let mut direct = backend
            .split_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circular_split_references(&mut direct, "m190-document", "m190-split").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == "extrusion.side(profile_edge=circle)"
                && reference.expected_type == "cylindrical_face"
        }));
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 20, 11, 2, 2], "corner {index}");

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            let reference = package.reference(role).unwrap();
            assert!(reference.has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory
            .path()
            .join(format!("circle-center-on-corner-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-center-on-corner-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        let imported_topology = [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ];
        assert_eq!(imported_topology, [16, 24, 12, 2, 2], "STEP corner {index}");
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([0.0, 1.0e-5], 10.0),
        ([0.0, 0.0], 60.0),
        ([-20.0, -20.0], 5.0),
        ([0.0, 0.0], 100.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
}

#[test]
fn scheduler_evaluates_strict_outside_center_circular_blind_pocket() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let distance = 5.0_f64;
    let pocket_depth = 7.0_f64;
    let chord_half = (radius * radius - distance * distance).sqrt();
    let overlap_area = radius * radius * (distance / radius).acos() - distance * chord_half;
    let expected_volume = 108_000.0 - overlap_area * pocket_depth;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, overlap_bounds, host_role)) in [
        (
            [-5.0, 30.0],
            [0.0, 30.0 - chord_half, 5.0, 30.0 + chord_half],
            ExactFaceRole::East,
        ),
        (
            [105.0, 30.0],
            [95.0, 30.0 - chord_half, 100.0, 30.0 + chord_half],
            ExactFaceRole::West,
        ),
        (
            [50.0, -5.0],
            [50.0 - chord_half, 0.0, 50.0 + chord_half, 5.0],
            ExactFaceRole::East,
        ),
        (
            [50.0, 65.0],
            [50.0 - chord_half, 55.0, 50.0 + chord_half, 60.0],
            ExactFaceRole::East,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_POCKET_EVALUATOR_V1);
        assert_eq!(request.pocket_depth_bits, Some(pocket_depth.to_bits()));
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        let (classified_area, classified_bounds) =
            circle.outside_side_overlap(100.0, 60.0).unwrap();
        assert!((classified_area - overlap_area).abs() < 1.0e-9);
        assert_eq!(classified_bounds, overlap_bounds);

        let mut direct = backend
            .cut_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 11.0,
                    radius_mm: radius,
                    height_mm: 8.0,
                },
                CutMode::BlindPlanar,
            )
            .unwrap();
        let direct_references = capture_circular_pocket_references(
            &mut direct,
            "m180-document",
            "m180-pocket",
            base_spec,
            11.0,
        )
        .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(direct_topology, [12, 18, 8, 1, 1], "orientation {index}");
        for semantic_role in [
            host_role.semantic_role(),
            ExactFaceRole::CutCircle.semantic_role(),
            ExactFaceRole::PocketFloor.semantic_role(),
        ] {
            assert!(
                direct_references
                    .iter()
                    .any(|reference| reference.semantic_role == semantic_role)
            );
        }

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            host_role,
            ExactFaceRole::CutCircle,
            ExactFaceRole::PocketFloor,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert!(package.vertices.iter().all(|vertex| {
            vertex.position_mm[0] >= -1.0e-9
                && vertex.position_mm[0] <= 100.0 + 1.0e-9
                && vertex.position_mm[1] >= -1.0e-9
                && vertex.position_mm[1] <= 60.0 + 1.0e-9
                && vertex.position_mm[2] >= -1.0e-9
                && vertex.position_mm[2] <= 18.0 + 1.0e-9
        }));
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 1);

        let step_path = directory
            .path()
            .join(format!("circle-outside-side-pocket-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-side-pocket-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        let imported_topology = [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ];
        assert_eq!(
            imported_topology,
            [12, 18, 8, 1, 1],
            "STEP orientation {index}"
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 1.0e-3
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

    for (center, radius) in [
        ([-10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([-5.0, 5.0], 10.0),
        ([50.0, 30.0], 70.0),
    ] {
        let rejected = circular_pocket_document(100.0, 60.0, 18.0, center, radius, pocket_depth);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    for invalid_depth in [18.0, 19.0] {
        let mut rejected =
            circular_pocket_document(100.0, 60.0, 18.0, [-5.0, 30.0], radius, pocket_depth);
        let before = rejected.current();
        assert!(
            rejected
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetFeatureDimension {
                        id: POCKET,
                        dimension: Dimension::new(invalid_depth.to_string(), invalid_depth)
                            .unwrap(),
                    },
                ]))
                .is_err()
        );
        assert_eq!(rejected.current().revision_id(), before.revision_id());
        assert_eq!(
            rejected.current().canonical_digest(),
            before.canonical_digest()
        );
    }
}

#[test]
fn scheduler_evaluates_strict_outside_center_circular_split() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 18.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let radius = 10.0_f64;
    let expected_volume = 100.0 * 60.0 * 18.0;
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let directory = tempfile::tempdir().unwrap();

    for (index, (center, expected_direct_topology, expected_step_topology)) in [
        ([-5.0, 30.0], [14, 23, 12, 2, 2], [20, 30, 14, 2, 2]),
        ([105.0, 30.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
        ([50.0, -5.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
        ([50.0, 65.0], [12, 20, 11, 2, 2], [16, 24, 12, 2, 2]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        let snapshot = document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [[0.0; 3], [100.0, 60.0, 18.0]]
        );
        let circle = request.boolean.as_ref().unwrap().circle.unwrap();
        assert_eq!(circle.side_overlap(100.0, 60.0), None);
        assert_eq!(circle.corner_overlap(100.0, 60.0), None);
        assert!(circle.outside_side_overlap(100.0, 60.0).is_some());

        let mut direct = backend
            .split_cylinder(
                &base.body,
                CylinderToolSpec {
                    center_mm: center,
                    origin_z_mm: 0.0,
                    radius_mm: radius,
                    height_mm: 18.0,
                },
            )
            .unwrap();
        assert!((direct.body.topology.volume_mm3 - expected_volume).abs() < 2.0e-3);
        let direct_references =
            capture_circular_split_references(&mut direct, "m179-document", "m179-split").unwrap();
        assert_eq!(direct_references.len(), 3);
        assert!(direct_references.iter().any(|reference| {
            reference.semantic_role == "extrusion.side(profile_edge=circle)"
                && reference.expected_type == "cylindrical_face"
        }));
        let direct_topology = [
            direct.body.topology.vertex_count,
            direct.body.topology.edge_count,
            direct.body.topology.face_count,
            direct.body.topology.shell_count,
            direct.body.topology.solid_count,
        ];
        assert_eq!(
            direct_topology, expected_direct_topology,
            "orientation {index}"
        );

        let package = supervisor.evaluate_rectangle(&request).unwrap();
        let repeated = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(
            package.identity.result_fingerprint,
            direct.body.result_fingerprint
        );
        assert_eq!(package.identity, repeated.identity);
        assert_eq!(package.references, repeated.references);
        assert_eq!(package.bounds_mm, request.expected_bounds_mm());
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::CircleSide,
        ] {
            assert!(package.reference(role).unwrap().has_valid_lineage());
            assert!(
                package
                    .triangles
                    .iter()
                    .any(|triangle| triangle.face_role == Some(role))
            );
        }
        assert_closed_manifold(&package);
        assert_consistently_oriented_closed_manifold(&package);
        assert_eq!(mesh_component_count(&package), 2);

        let step_path = directory
            .path()
            .join(format!("circle-outside-side-split-{index}.step"));
        let stale_path = directory
            .path()
            .join(format!("stale-circle-outside-side-split-{index}.step"));
        let model = [(ExactBodyPackage::from(package), Transform::identity())];
        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        supervisor
            .export_current_model_step(&snapshot, &model, &step_path)
            .unwrap();
        let imported = backend.import_step(step_path.to_str().unwrap()).unwrap();
        let imported_topology = [
            imported.body.topology.vertex_count,
            imported.body.topology.edge_count,
            imported.body.topology.face_count,
            imported.body.topology.shell_count,
            imported.body.topology.solid_count,
        ];
        assert_eq!(
            imported_topology, expected_step_topology,
            "STEP orientation {index}"
        );
        assert!(
            (imported.body.topology.volume_mm3 - expected_volume).abs() / expected_volume < 5.0e-3
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

    for (center, radius) in [
        ([-10.0, 30.0], 10.0),
        ([-20.0, 30.0], 5.0),
        ([-5.0, 5.0], 10.0),
    ] {
        let rejected =
            circular_boolean_document(100.0, 60.0, 18.0, center, radius, BooleanOperation::Split);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
    }
    let pocket = circular_pocket_document(100.0, 60.0, 18.0, [-5.0, 30.0], radius, 7.0);
    assert!(ExactFeatureChainRequest::from_snapshot(&pocket.current(), DEFINITION).is_ok());
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

    for (center, radius) in [([10.0, 30.0], 10.0), ([120.0, 30.0], 5.0)] {
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

    for (center, radius, operation) in [
        ([10.0, 30.0], 10.0, BooleanOperation::Intersect),
        ([120.0, 30.0], 5.0, BooleanOperation::Intersect),
        ([10.0, 30.0], 10.0, BooleanOperation::Split),
        ([120.0, 30.0], 5.0, BooleanOperation::Split),
    ] {
        let rejected = circular_boolean_document(100.0, 60.0, 18.0, center, radius, operation);
        assert!(ExactFeatureChainRequest::from_snapshot(&rejected.current(), DEFINITION).is_err());
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
    let mut boundary_request = None;
    let mut boundary_package = None;
    for distance_mm in [5.0, -7.5, 0.01] {
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
        if distance_mm == 0.01 {
            boundary_request = Some(request.clone());
        }
        assert_eq!(request.producer_feature_id(), OFFSET_FEATURE);
        assert_eq!(request.evaluator(), EXACT_PLANAR_OFFSET_EVALUATOR_V1);
        assert_eq!(
            request.expected_bounds_mm(),
            [
                [10.0 - distance_mm, 20.0 - distance_mm, 0.0],
                [110.0 + distance_mm, 100.0 + distance_mm, 0.0],
            ]
        );

        let first = supervisor.evaluate_planar_offset(&request).unwrap();
        let repeated = supervisor.evaluate_planar_offset(&request).unwrap();
        if distance_mm == 0.01 {
            boundary_package = Some(first.clone());
        }
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

    let mut below_minimum = boundary_request
        .clone()
        .expect("minimum offset request was evaluated");
    below_minimum.distance_bits = 0.009_f64.to_bits();
    assert!(matches!(
        supervisor.evaluate_planar_offset(&below_minimum),
        Err(M3EvaluationError::Worker(WorkerError::Geometry(code)))
            if code == GeometryErrorCode::InvalidParameter.as_str()
    ));

    let package = boundary_package.expect("minimum offset package was evaluated");
    let mut non_finite_request = boundary_request.expect("minimum offset request was evaluated");
    non_finite_request.distance_bits = f64::NAN.to_bits();
    assert!(matches!(
        build_planar_offset_package(
            &non_finite_request,
            PlanarOffsetWorkerEvidence {
                exact_input_digest: package.identity.exact_input_digest,
                result_fingerprint: package.identity.result_fingerprint,
                backend: package.identity.backend,
                tolerance: package.identity.tolerance,
                bounds_mm: package.bounds_mm,
                area_mm2: package.area_mm2,
                topology_counts: package.topology_counts,
                face_ordinal: 0,
                lineage_digest: package.reference.lineage_digest,
                corroborating_geometry_fingerprint: package
                    .reference
                    .corroborating_geometry_fingerprint,
            },
        ),
        Err(ExactProductError::InvalidWorkerEvidence)
    ));
}

#[test]
fn scheduler_evaluates_circular_planar_offset_with_bounded_worker_evidence() {
    const DEFINITION: DefinitionId = DefinitionId(704);
    const WORKPLANE: FeatureId = FeatureId(705);
    const CIRCLE: FeatureId = FeatureId(706);
    const OFFSET: FeatureId = FeatureId(707);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut previous_fingerprint = None;
    for (distance_mm, output_radius_mm) in [(3.0, 23.0), (-3.0, 17.0), (-19.99, 0.01)] {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Circular offset".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: WORKPLANE,
                    definition_id: DEFINITION,
                    name: "XY".to_owned(),
                    kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                },
                CanonicalCommand::CreateFeature {
                    id: CIRCLE,
                    definition_id: DEFINITION,
                    name: "Circle sketch".to_owned(),
                    kind: FeatureKind::Sketch(SketchSpec {
                        workplane: WORKPLANE,
                        entities: vec![SketchEntity::Circle {
                            id: SketchEntityId(1),
                            center_mm: [12.0, -8.0],
                            radius_mm: 20.0,
                        }],
                        constraints: Vec::new(),
                    }),
                },
                CanonicalCommand::CreateFeature {
                    id: OFFSET,
                    definition_id: DEFINITION,
                    name: "Planar offset".to_owned(),
                    kind: FeatureKind::PlanarOffset {
                        profile: CIRCLE,
                        distance: Dimension::new(distance_mm.to_string(), distance_mm).unwrap(),
                    },
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let request = ExactPlanarOffsetRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let circle = request.circle_profile().unwrap();
        assert!(!request.is_rectangle());
        assert!(request.mixed_profile().is_none());
        assert_eq!(f64::from_bits(circle.center_x_bits), 12.0);
        assert_eq!(f64::from_bits(circle.center_y_bits), -8.0);
        assert_eq!(f64::from_bits(circle.radius_bits), 20.0);

        let first = supervisor.evaluate_planar_offset(&request).unwrap();
        let repeated = supervisor.evaluate_planar_offset(&request).unwrap();
        assert!(first.is_current(&snapshot));
        assert_eq!(first.identity, repeated.identity);
        assert_eq!(first.reference, repeated.reference);
        assert_eq!(first.topology_counts, [1, 1, 1, 0, 0]);
        assert!(first.vertices.is_empty());
        assert!(first.triangles.is_empty());
        let expected_bounds = [
            [12.0 - output_radius_mm, -8.0 - output_radius_mm, 0.0],
            [12.0 + output_radius_mm, -8.0 + output_radius_mm, 0.0],
        ];
        assert!(
            first
                .bounds_mm
                .into_iter()
                .flatten()
                .zip(expected_bounds.into_iter().flatten())
                .all(|(actual, expected)| (actual - expected).abs() <= 1.0e-6)
        );
        assert!(
            (first.area_mm2 - std::f64::consts::PI * output_radius_mm * output_radius_mm).abs()
                <= 1.0e-6 * first.area_mm2.max(1.0)
        );
        assert_eq!(
            first.reference.role(),
            Some(ExactFaceRole::PlanarOffsetFace)
        );
        assert!(first.reference.has_valid_lineage());
        assert!(first.reference.matches_planar_offset_request(&request));

        let evidence = PlanarOffsetWorkerEvidence {
            exact_input_digest: first.identity.exact_input_digest.clone(),
            result_fingerprint: first.identity.result_fingerprint.clone(),
            backend: first.identity.backend.clone(),
            tolerance: first.identity.tolerance.clone(),
            bounds_mm: first.bounds_mm,
            area_mm2: first.area_mm2,
            topology_counts: first.topology_counts,
            face_ordinal: 0,
            lineage_digest: first.reference.lineage_digest.clone(),
            corroborating_geometry_fingerprint: first
                .reference
                .corroborating_geometry_fingerprint
                .clone(),
        };
        let mut tampered = request.clone();
        tampered.circle.as_mut().unwrap().radius_bits = 21.0_f64.to_bits();
        assert!(matches!(
            build_planar_offset_package(&tampered, evidence),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
        if let Some(previous_fingerprint) = previous_fingerprint {
            assert_ne!(first.identity.result_fingerprint, previous_fingerprint);
        }
        previous_fingerprint = Some(first.identity.result_fingerprint);
    }

    for invalid_distance in [-20.0, -19.991] {
        let mut invalid = DocumentStore::new();
        assert!(matches!(
            invalid.apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Invalid circular offset".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: WORKPLANE,
                    definition_id: DEFINITION,
                    name: "XY".to_owned(),
                    kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                },
                CanonicalCommand::CreateFeature {
                    id: CIRCLE,
                    definition_id: DEFINITION,
                    name: "Circle sketch".to_owned(),
                    kind: FeatureKind::Sketch(SketchSpec {
                        workplane: WORKPLANE,
                        entities: vec![SketchEntity::Circle {
                            id: SketchEntityId(1),
                            center_mm: [12.0, -8.0],
                            radius_mm: 20.0,
                        }],
                        constraints: Vec::new(),
                    }),
                },
                CanonicalCommand::CreateFeature {
                    id: OFFSET,
                    definition_id: DEFINITION,
                    name: "Collapsed offset".to_owned(),
                    kind: FeatureKind::PlanarOffset {
                        profile: CIRCLE,
                        distance: Dimension::new(invalid_distance.to_string(), invalid_distance,)
                            .unwrap(),
                    },
                },
            ])),
            Err(CanonicalError::InvalidPlanarOffset)
        ));
        assert_eq!(invalid.visible_undo_steps(), 0);
    }
}

#[test]
fn scheduler_evaluates_typed_line_arc_planar_offset_with_worker_evidence() {
    const DEFINITION: DefinitionId = DefinitionId(704);
    const PROFILE: FeatureId = FeatureId(705);
    const OFFSET: FeatureId = FeatureId(706);

    let segments = vec![
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
    ];
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut fingerprints = Vec::new();
    for distance_mm in [2.0, -2.0, -4.9] {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Typed offset".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: PROFILE,
                    definition_id: DEFINITION,
                    name: "Line arc profile".to_owned(),
                    kind: FeatureKind::SegmentProfile {
                        segments: segments.clone(),
                        closed: true,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: OFFSET,
                    definition_id: DEFINITION,
                    name: "Planar offset".to_owned(),
                    kind: FeatureKind::PlanarOffset {
                        profile: PROFILE,
                        distance: Dimension::new(distance_mm.to_string(), distance_mm).unwrap(),
                    },
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let request = ExactPlanarOffsetRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        assert!(!request.is_rectangle());
        assert_eq!(request.mixed_profile().unwrap().segments.len(), 2);

        let first = supervisor.evaluate_planar_offset(&request).unwrap();
        let repeated = supervisor.evaluate_planar_offset(&request).unwrap();
        assert!(first.is_current(&snapshot));
        assert_eq!(first.identity, repeated.identity);
        assert_eq!(first.bounds_mm, repeated.bounds_mm);
        assert_eq!(first.area_mm2, repeated.area_mm2);
        assert_eq!(first.reference, repeated.reference);
        assert_eq!(first.topology_counts[2..], [1, 0, 0]);
        assert_eq!(first.topology_counts[0], first.topology_counts[1]);
        assert!(first.vertices.is_empty());
        assert!(first.triangles.is_empty());
        assert_eq!(
            first.reference.role(),
            Some(ExactFaceRole::PlanarOffsetFace)
        );
        assert!(first.reference.has_valid_lineage());
        assert!(first.reference.matches_planar_offset_request(&request));

        let evidence = PlanarOffsetWorkerEvidence {
            exact_input_digest: first.identity.exact_input_digest.clone(),
            result_fingerprint: first.identity.result_fingerprint.clone(),
            backend: first.identity.backend.clone(),
            tolerance: first.identity.tolerance.clone(),
            bounds_mm: first.bounds_mm,
            area_mm2: first.area_mm2,
            topology_counts: first.topology_counts,
            face_ordinal: 0,
            lineage_digest: first.reference.lineage_digest.clone(),
            corroborating_geometry_fingerprint: first
                .reference
                .corroborating_geometry_fingerprint
                .clone(),
        };
        let mut tampered_bounds = request.clone();
        tampered_bounds.source_bounds_bits[0] = 1.0_f64.to_bits();
        assert!(matches!(
            build_planar_offset_package(&tampered_bounds, evidence.clone()),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
        let mut unrelated_geometry = evidence;
        unrelated_geometry.bounds_mm = [[100.0, 100.0, 0.0], [120.0, 120.0, 0.0]];
        assert!(matches!(
            build_planar_offset_package(&request, unrelated_geometry),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));

        fingerprints.push(first.identity.result_fingerprint);

        let mut tampered = request.clone();
        tampered.profile.as_mut().unwrap().segments.pop();
        assert!(supervisor.evaluate_planar_offset(&tampered).is_err());
    }
    assert_ne!(fingerprints[0], fingerprints[1]);

    let mut diamond = DocumentStore::new();
    diamond
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Mitered offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Diamond profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: [[0.0, -10.0], [10.0, 0.0], [0.0, 10.0], [-10.0, 0.0]]
                        .into_iter()
                        .zip([[10.0, 0.0], [0.0, 10.0], [-10.0, 0.0], [0.0, -10.0]])
                        .map(|(start_mm, end_mm)| ProfileSegment::Line { start_mm, end_mm })
                        .collect(),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Mitered planar offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: PROFILE,
                    distance: Dimension::new("2", 2.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let diamond_snapshot = diamond.current();
    let diamond_request =
        ExactPlanarOffsetRequest::from_snapshot(&diamond_snapshot, DEFINITION).unwrap();
    let diamond_result = supervisor.evaluate_planar_offset(&diamond_request).unwrap();
    assert!(diamond_result.bounds_mm[0][0] < -12.0);
    assert!(diamond_result.bounds_mm[0][1] < -12.0);
    assert!(diamond_result.bounds_mm[1][0] > 12.0);
    assert!(diamond_result.bounds_mm[1][1] > 12.0);

    let mut invalid = DocumentStore::new();
    assert!(matches!(
        invalid.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Open typed offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Open profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Invalid offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: PROFILE,
                    distance: Dimension::new("2", 2.0).unwrap(),
                },
            },
        ])),
        Err(CanonicalError::InvalidPlanarOffset)
    ));

    let self_intersecting = vec![
        ProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 20.0],
        },
        ProfileSegment::Line {
            start_mm: [20.0, 20.0],
            end_mm: [0.0, 20.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [0.0, 20.0],
            end_mm: [20.0, 0.0],
            center_mm: [10.0, 10.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [20.0, 0.0],
            end_mm: [0.0, 0.0],
        },
    ];
    let mut invalid = DocumentStore::new();
    assert!(matches!(
        invalid.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Self-intersecting typed offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Self-intersecting profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: self_intersecting,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Invalid offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: PROFILE,
                    distance: Dimension::new("2", 2.0).unwrap(),
                },
            },
        ])),
        Err(CanonicalError::InvalidPlanarOffset)
    ));
}

#[test]
fn scheduler_evaluates_signed_mixed_cubic_planar_offset_over_v2() {
    const DEFINITION: DefinitionId = DefinitionId(707);
    const WORKPLANE: FeatureId = FeatureId(708);
    const SKETCH: FeatureId = FeatureId(709);
    const OFFSET: FeatureId = FeatureId(710);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut fingerprints = Vec::new();
    for distance_mm in [2.0, -1.0] {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Mixed cubic offset".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: WORKPLANE,
                    definition_id: DEFINITION,
                    name: "XY".to_owned(),
                    kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                },
                CanonicalCommand::CreateFeature {
                    id: SKETCH,
                    definition_id: DEFINITION,
                    name: "Line arc cubic profile".to_owned(),
                    kind: FeatureKind::Sketch(SketchSpec {
                        workplane: WORKPLANE,
                        entities: vec![
                            SketchEntity::Line {
                                id: SketchEntityId(1),
                                start_mm: [0.0, 0.0],
                                end_mm: [40.0, 0.0],
                            },
                            SketchEntity::Arc {
                                id: SketchEntityId(2),
                                start_mm: [40.0, 0.0],
                                end_mm: [50.0, 10.0],
                                center_mm: [40.0, 10.0],
                                clockwise: false,
                            },
                            SketchEntity::CubicBezier {
                                id: SketchEntityId(3),
                                start_mm: [50.0, 10.0],
                                control_1_mm: [50.0, 20.0],
                                control_2_mm: [0.0, 20.0],
                                end_mm: [0.0, 10.0],
                            },
                            SketchEntity::Line {
                                id: SketchEntityId(4),
                                start_mm: [0.0, 10.0],
                                end_mm: [0.0, 0.0],
                            },
                        ],
                        constraints: Vec::new(),
                    }),
                },
                CanonicalCommand::CreateFeature {
                    id: OFFSET,
                    definition_id: DEFINITION,
                    name: "Signed mixed offset".to_owned(),
                    kind: FeatureKind::PlanarOffset {
                        profile: SKETCH,
                        distance: Dimension::new(distance_mm.to_string(), distance_mm).unwrap(),
                    },
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let request = ExactPlanarOffsetRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let segments = &request.mixed_profile().unwrap().segments;
        assert!(
            segments
                .iter()
                .any(|segment| matches!(segment, ExactBRepPlanarSegment::Line { .. }))
        );
        assert!(
            segments
                .iter()
                .any(|segment| matches!(segment, ExactBRepPlanarSegment::CircularArc { .. }))
        );
        assert!(
            segments
                .iter()
                .any(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
        );

        let first = supervisor.evaluate_planar_offset(&request).unwrap();
        let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, OFFSET).unwrap();
        assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V7);
        let graph_result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(
            graph_result.identity.result_fingerprint,
            first.identity.result_fingerprint
        );
        assert_eq!(graph_result.bounds_mm, first.bounds_mm);
        assert_eq!(graph_result.area_mm2, first.area_mm2);
        assert!(!graph_result.vertices.is_empty());
        assert!(!graph_result.triangles.is_empty());
        let repeated = supervisor.evaluate_planar_offset(&request).unwrap();
        assert_eq!(first, repeated);
        assert!(first.is_current(&snapshot));
        assert_eq!(first.topology_counts[2..], [1, 0, 0]);
        assert_eq!(first.topology_counts[0], first.topology_counts[1]);
        assert!(first.reference.has_valid_lineage());
        assert!(first.reference.matches_planar_offset_request(&request));
        fingerprints.push(first.identity.result_fingerprint);

        let mut forged = request.clone();
        let cubic = forged
            .profile
            .as_mut()
            .unwrap()
            .segments
            .iter_mut()
            .find(|segment| matches!(segment, ExactBRepPlanarSegment::CubicBezier { .. }))
            .unwrap();
        let ExactBRepPlanarSegment::CubicBezier { control_2_bits, .. } = cubic else {
            unreachable!();
        };
        control_2_bits[0] = 21.0_f64.to_bits();
        assert!(supervisor.evaluate_planar_offset(&forged).is_err());
    }
    assert_ne!(fingerprints[0], fingerprints[1]);
}

#[test]
fn scheduler_evaluates_bounded_sweep_and_rejects_path_length_parity() {
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
    assert_eq!(MIN_EXACT_BREP_SWEEP_PATH_LENGTH_MM, 0.01);
    assert_eq!(MAX_EXACT_BREP_SWEEP_PATH_LENGTH_MM, 100_000.0);

    for invalid_length in [0.009, 100_000.001] {
        let mut invalid_document = DocumentStore::new();
        let empty = invalid_document.current().canonical_digest();
        let error = invalid_document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Invalid Sweep definition".to_owned(),
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
                    name: "Out-of-envelope path".to_owned(),
                    kind: FeatureKind::SegmentProfile {
                        segments: vec![ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [invalid_length, 0.0],
                        }],
                        closed: false,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: SWEEP,
                    definition_id: DEFINITION,
                    name: "Invalid Sweep".to_owned(),
                    kind: FeatureKind::Sweep {
                        profile: PROFILE,
                        path: PATH,
                    },
                },
            ]))
            .err()
            .expect("canonical Sweep must reject an out-of-envelope path before worker evaluation");
        assert_eq!(error, CanonicalError::InvalidSweep);
        assert_eq!(invalid_document.current().canonical_digest(), empty);
        assert_eq!(invalid_document.visible_undo_steps(), 0);
    }

    let mut output_limit_document = DocumentStore::new();
    output_limit_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Sweep output envelope".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangular section".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Boundary path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [1_000_000.0, 0.0],
                        end_mm: [1_000_000.0, 100.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Out-of-envelope Sweep".to_owned(),
                kind: FeatureKind::Sweep {
                    profile: PROFILE,
                    path: PATH,
                },
            },
        ]))
        .unwrap();
    assert_eq!(
        ExactSweepRequest::from_snapshot(&output_limit_document.current(), DEFINITION),
        Err(ExactProductError::UnsupportedProfile)
    );

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let first = supervisor.evaluate_sweep(&request).unwrap();
    let repeated = supervisor.evaluate_sweep(&request).unwrap();
    for invalid_length in [0.009, 100_000.001] {
        let mut invalid_request = request.clone();
        invalid_request.path_bits = [0.0, 0.0, invalid_length, 0.0].map(f64::to_bits);
        assert!(matches!(
            supervisor.evaluate_sweep(&invalid_request),
            Err(M3EvaluationError::Worker(WorkerError::Geometry(code)))
                if code == GeometryErrorCode::InvalidParameter.as_str()
        ));
    }
    let mut output_limit_request = request.clone();
    output_limit_request.profile_bounds_bits = [0.0, 0.0, 10.0, 10.0].map(f64::to_bits);
    output_limit_request.path_bits = [1_000_000.0, 0.0, 1_000_000.0, 100.0].map(f64::to_bits);
    assert!(matches!(
        supervisor.evaluate_sweep(&output_limit_request),
        Err(M3EvaluationError::Worker(WorkerError::Geometry(code)))
            if code == GeometryErrorCode::InvalidParameter.as_str()
    ));
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
fn scheduler_evaluates_bounded_spline_loft_and_rejects_over_limit_parity() {
    const DEFINITION: DefinitionId = DefinitionId(721);
    const LOWER: FeatureId = FeatureId(722);
    const UPPER: FeatureId = FeatureId(723);
    const LOFT: FeatureId = FeatureId(724);

    let boundary_points = |radius_x: f64, radius_y: f64| {
        (0..MAX_EXACT_BREP_LOFT_CONTROL_POINTS)
            .map(|index| {
                let angle = std::f64::consts::TAU * index as f64
                    / MAX_EXACT_BREP_LOFT_CONTROL_POINTS as f64;
                [radius_x * angle.cos(), radius_y * angle.sin()]
            })
            .collect::<Vec<_>>()
    };
    let lower_points = boundary_points(20.0, 10.0);
    let upper_points = boundary_points(10.0, 5.0);
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
                    control_points_mm: lower_points.clone(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: UPPER,
                definition_id: DEFINITION,
                name: "Upper spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: upper_points.clone(),
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
    let before_invalid_elevation = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(725),
            definition_id: DEFINITION,
            name: "Out-of-envelope Loft".to_owned(),
            kind: FeatureKind::Loft {
                sections: vec![
                    LoftSection {
                        profile: LOWER,
                        elevation_mm: 0.0,
                    },
                    LoftSection {
                        profile: UPPER,
                        elevation_mm: 1_000_000.001,
                    },
                ],
            },
        }]))
        .err()
        .expect("canonical Loft must reject an out-of-envelope elevation");
    assert_eq!(error, CanonicalError::InvalidLoft);
    assert_eq!(
        document.current().canonical_digest(),
        before_invalid_elevation
    );
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let snapshot = document.current();
    let request = ExactLoftRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    assert_eq!(request.producer_feature_id(), LOFT);
    assert_eq!(request.evaluator(), EXACT_LOFT_EVALUATOR_V1);
    assert_eq!(request.control_point_count(), 128);
    assert_eq!(request.protocol_values().len(), 261);

    let over_limit_points = (0..=MAX_EXACT_BREP_LOFT_CONTROL_POINTS)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64
                / (MAX_EXACT_BREP_LOFT_CONTROL_POINTS + 1) as f64;
            [20.0 * angle.cos(), 10.0 * angle.sin()]
        })
        .collect::<Vec<_>>();
    let mut over_limit_document = DocumentStore::new();
    let empty = over_limit_document.current().canonical_digest();
    let error = over_limit_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Over-limit Loft definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LOWER,
                definition_id: DEFINITION,
                name: "Over-limit lower spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: over_limit_points.clone(),
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
                name: "Over-limit Loft".to_owned(),
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
        .err()
        .expect("canonical Loft must reject a 65-point section before worker evaluation");
    assert_eq!(error, CanonicalError::InvalidLoft);
    assert_eq!(over_limit_document.current().canonical_digest(), empty);
    assert_eq!(over_limit_document.visible_undo_steps(), 0);

    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let first = supervisor.evaluate_loft(&request).unwrap();
    let repeated = supervisor.evaluate_loft(&request).unwrap();
    let mut over_limit_request = request.clone();
    over_limit_request.sections[0].control_point_bits = over_limit_points
        .iter()
        .map(|point| point.map(f64::to_bits))
        .collect();
    assert!(matches!(
        supervisor.evaluate_loft(&over_limit_request),
        Err(M3EvaluationError::Worker(WorkerError::Geometry(code)))
            if code == GeometryErrorCode::InvalidParameter.as_str()
    ));
    let mut over_limit_request = request.clone();
    over_limit_request.sections[1].elevation_bits = 1_000_000.001_f64.to_bits();
    assert!(matches!(
        supervisor.evaluate_loft(&over_limit_request),
        Err(M3EvaluationError::Worker(WorkerError::Geometry(code)))
            if code == GeometryErrorCode::InvalidParameter.as_str()
    ));
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

fn try_circular_pocket_document(
    width: f64,
    depth: f64,
    height: f64,
    center: [f64; 2],
    radius: f64,
    pocket_depth: f64,
) -> Result<DocumentStore, CanonicalError> {
    let mut document = rectangle_document(width, depth, height);
    document.apply_batch(&CommandBatch::new(vec![
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
    ]))?;
    document.discard_history_before_current();
    Ok(document)
}

fn circular_pocket_document(
    width: f64,
    depth: f64,
    height: f64,
    center: [f64; 2],
    radius: f64,
    pocket_depth: f64,
) -> DocumentStore {
    try_circular_pocket_document(width, depth, height, center, radius, pocket_depth).unwrap()
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

fn mesh_component_count(package: &ExactRenderPackage) -> usize {
    let mut adjacency = vec![Vec::new(); package.vertices.len()];
    let mut used = vec![false; package.vertices.len()];
    for triangle in &package.triangles {
        for [from, to] in [
            [triangle.vertex_indices[0], triangle.vertex_indices[1]],
            [triangle.vertex_indices[1], triangle.vertex_indices[2]],
            [triangle.vertex_indices[2], triangle.vertex_indices[0]],
        ] {
            adjacency[from as usize].push(to as usize);
            adjacency[to as usize].push(from as usize);
            used[from as usize] = true;
            used[to as usize] = true;
        }
    }
    let mut visited = vec![false; package.vertices.len()];
    let mut components = 0;
    for start in 0..package.vertices.len() {
        if !used[start] || visited[start] {
            continue;
        }
        components += 1;
        visited[start] = true;
        let mut pending = vec![start];
        while let Some(vertex) = pending.pop() {
            for &neighbor in &adjacency[vertex] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    pending.push(neighbor);
                }
            }
        }
    }
    components
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
    line_arc_d_boolean_document_with_segments(height, operation, segments)
}

fn line_arc_d_boolean_document_with_segments(
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

fn translated_capsule_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    capsule_segments()
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

fn rotated_capsule_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    capsule_segments()
        .into_iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
                start_mm: [-start_mm[1] + dx, start_mm[0] + dy],
                end_mm: [-end_mm[1] + dx, end_mm[0] + dy],
            },
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => ProfileSegment::CircularArc {
                start_mm: [-start_mm[1] + dx, start_mm[0] + dy],
                end_mm: [-end_mm[1] + dx, end_mm[0] + dy],
                center_mm: [-center_mm[1] + dx, center_mm[0] + dy],
                clockwise,
            },
        })
        .collect()
}

fn translated_capsule_planar_segments(dx: f64) -> Vec<PlanarProfileSegment> {
    capsule_planar_segments()
        .into_iter()
        .map(|segment| match segment {
            PlanarProfileSegment::Line {
                mut start_mm,
                mut end_mm,
            } => {
                start_mm[0] += dx;
                end_mm[0] += dx;
                PlanarProfileSegment::Line { start_mm, end_mm }
            }
            PlanarProfileSegment::CircularArc {
                mut start_mm,
                mut end_mm,
                mut center_mm,
                clockwise,
            } => {
                start_mm[0] += dx;
                end_mm[0] += dx;
                center_mm[0] += dx;
                PlanarProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                }
            }
            PlanarProfileSegment::CubicBezier {
                mut start_mm,
                mut control_1_mm,
                mut control_2_mm,
                mut end_mm,
            } => {
                start_mm[0] += dx;
                control_1_mm[0] += dx;
                control_2_mm[0] += dx;
                end_mm[0] += dx;
                PlanarProfileSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                }
            }
        })
        .collect()
}

fn asymmetric_convex_mixed_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    let point = |x: f64, y: f64| [x + dx, y + dy];
    vec![
        ProfileSegment::Line {
            start_mm: point(25.0, 15.0),
            end_mm: point(65.0, 15.0),
        },
        ProfileSegment::CircularArc {
            start_mm: point(65.0, 15.0),
            end_mm: point(75.0, 25.0),
            center_mm: point(65.0, 25.0),
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: point(75.0, 25.0),
            end_mm: point(65.0, 45.0),
        },
        ProfileSegment::Line {
            start_mm: point(65.0, 45.0),
            end_mm: point(25.0, 45.0),
        },
        ProfileSegment::Line {
            start_mm: point(25.0, 45.0),
            end_mm: point(25.0, 15.0),
        },
    ]
}

fn asymmetric_convex_mixed_planar_segments(dx: f64, dy: f64) -> Vec<PlanarProfileSegment> {
    planar_segments(asymmetric_convex_mixed_segments(dx, dy))
}

fn arc_only_clipped_asymmetric_convex_mixed_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    let point = |x: f64, y: f64| [x + dx, y + dy];
    vec![
        ProfileSegment::Line {
            start_mm: point(40.0, 15.0),
            end_mm: point(90.0, 15.0),
        },
        ProfileSegment::CircularArc {
            start_mm: point(90.0, 15.0),
            end_mm: point(90.0, 45.0),
            center_mm: point(90.0, 30.0),
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: point(90.0, 45.0),
            end_mm: point(40.0, 45.0),
        },
        ProfileSegment::Line {
            start_mm: point(40.0, 45.0),
            end_mm: point(30.0, 30.0),
        },
        ProfileSegment::Line {
            start_mm: point(30.0, 30.0),
            end_mm: point(40.0, 15.0),
        },
    ]
}

fn arc_only_clipped_asymmetric_convex_mixed_planar_segments(
    dx: f64,
    dy: f64,
) -> Vec<PlanarProfileSegment> {
    planar_segments(arc_only_clipped_asymmetric_convex_mixed_segments(dx, dy))
}

fn containing_asymmetric_convex_mixed_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    let point = |x: f64, y: f64| [x + dx, y + dy];
    vec![
        ProfileSegment::Line {
            start_mm: point(-10.0, -10.0),
            end_mm: point(110.0, -10.0),
        },
        ProfileSegment::CircularArc {
            start_mm: point(110.0, -10.0),
            end_mm: point(140.0, 20.0),
            center_mm: point(110.0, 20.0),
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: point(140.0, 20.0),
            end_mm: point(110.0, 80.0),
        },
        ProfileSegment::Line {
            start_mm: point(110.0, 80.0),
            end_mm: point(-10.0, 80.0),
        },
        ProfileSegment::Line {
            start_mm: point(-10.0, 80.0),
            end_mm: point(-10.0, -10.0),
        },
    ]
}

fn containing_asymmetric_convex_mixed_planar_segments(
    dx: f64,
    dy: f64,
) -> Vec<PlanarProfileSegment> {
    planar_segments(containing_asymmetric_convex_mixed_segments(dx, dy))
}

fn concave_mixed_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [25.0, 15.0],
            end_mm: [65.0, 15.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [65.0, 15.0],
            end_mm: [75.0, 25.0],
            center_mm: [65.0, 25.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [75.0, 25.0],
            end_mm: [45.0, 25.0],
        },
        ProfileSegment::Line {
            start_mm: [45.0, 25.0],
            end_mm: [65.0, 45.0],
        },
        ProfileSegment::Line {
            start_mm: [65.0, 45.0],
            end_mm: [25.0, 45.0],
        },
        ProfileSegment::Line {
            start_mm: [25.0, 45.0],
            end_mm: [25.0, 15.0],
        },
    ]
}

fn concave_mixed_planar_segments() -> Vec<PlanarProfileSegment> {
    planar_segments(concave_mixed_segments())
}

fn self_intersecting_mixed_segments() -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::Line {
            start_mm: [25.0, 15.0],
            end_mm: [65.0, 15.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [65.0, 15.0],
            end_mm: [75.0, 25.0],
            center_mm: [65.0, 25.0],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [75.0, 25.0],
            end_mm: [25.0, 45.0],
        },
        ProfileSegment::Line {
            start_mm: [25.0, 45.0],
            end_mm: [65.0, 45.0],
        },
        ProfileSegment::Line {
            start_mm: [65.0, 45.0],
            end_mm: [25.0, 15.0],
        },
    ]
}

fn planar_segments(segments: Vec<ProfileSegment>) -> Vec<PlanarProfileSegment> {
    segments
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

fn translated_rounded_rectangle_segments(dx: f64, dy: f64) -> Vec<ProfileSegment> {
    rounded_rectangle_segments()
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

fn translated_rounded_rectangle_planar_segments(dx: f64, dy: f64) -> Vec<PlanarProfileSegment> {
    translated_rounded_rectangle_segments(dx, dy)
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

fn mixed_profile_pocket_document(
    segments: Vec<ProfileSegment>,
    height: f64,
    pocket_depth: f64,
) -> DocumentStore {
    let mut document = rectangle_document(100.0, 60.0, height);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Mixed pocket profile".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Mixed pocket".to_owned(),
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
        ExactFaceRole::West => (
            Vec3::new(-10.0, depth / 2.0, height / 2.0),
            Vec3::new(1.0, 0.0, 0.0),
        ),
        ExactFaceRole::CircleSide
        | ExactFaceRole::ArcSide
        | ExactFaceRole::LinearSide
        | ExactFaceRole::CutCircle
        | ExactFaceRole::CutLinear
        | ExactFaceRole::CutArc
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
