use ketchup_core::document::{
    BooleanOperation, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension,
    DocumentStore, EdgeFinishKind, FeatureId, FeatureKind, LoftSection, NodeId, ProfileSegment,
    SolidToolPlan, Transform,
};
use ketchup_core::exact_brep_graph::{
    ExactBRepGraph, ExactBRepPlanarGeometry, ExactBRepPlanarLoop, ExactBRepPlanarSegment,
    MAX_EXACT_BREP_GRAPH_PROFILES,
};
use ketchup_core::exact_product::{
    ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactFaceRole,
    ExactFeatureChainRequest, ExactPlanarOffsetRequest, ExactProductError, ExactResultRegistry,
};
use ketchup_core::graph::sha256_hex;
use ketchup_core::import::{StepImportMesh, StepMeshTriangle, plan_step_import};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, PadSpec, PocketSpec, PrincipalPlane,
    SketchEntity, SketchEntityId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_core::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceStability,
};
use ketchup_scheduler::{
    DerivedResult, EvaluationScheduler, ExactWorkerSupervisor, InsertOutcome, WorkerError,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn assert_bounds_close(actual: [[f64; 3]; 2], expected: [f64; 6]) {
    for (actual, expected) in actual.into_iter().flatten().zip(expected) {
        assert!((actual - expected).abs() <= 2.0e-7);
    }
}

fn assert_geometry_error(error: WorkerError, code: &str) {
    match error {
        WorkerError::Geometry(detail) => assert!(detail.starts_with(code), "{detail}"),
        other => panic!("expected {code} geometry refusal, got {other:?}"),
    }
}

fn simple_extrusion_graph() -> ExactBRepGraph {
    let definition = DefinitionId(90);
    let profile = FeatureId(900);
    let extrusion = FeatureId(901);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Safety graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Safety profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [8.0, 0.0], [8.0, 6.0], [0.0, 6.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: extrusion,
                definition_id: definition,
                name: "Safety extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(5.0),
                },
            },
        ]))
        .unwrap();
    ExactBRepGraph::from_snapshot(&document.current(), definition, extrusion).unwrap()
}

fn generated_boolean_scales() -> Vec<[f64; 3]> {
    let mut samples = vec![[0.5, 0.75, 0.6], [1.0, 1.0, 1.0], [3.0, 2.5, 1.75]];
    let mut state = 0x4558_4143_5420_2026_u64;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 11) as f64) / ((1_u64 << 53) as f64)
    };
    samples.extend((0..3).map(|_| [0.5 + next() * 2.5, 0.5 + next() * 2.0, 0.5 + next() * 1.25]));
    samples
}

fn rigid_profile_variant(points: &[[f64; 2]], variant: usize) -> Vec<[f64; 2]> {
    points
        .iter()
        .map(|[x, y]| match variant {
            0 => [*x, *y],
            1 => [*x + 37.0, *y - 23.0],
            2 => [-*y + 11.0, *x + 29.0],
            _ => unreachable!("property harness has exactly three rigid variants"),
        })
        .collect()
}

fn generated_boolean_document(
    scales: [f64; 3],
    rigid_variant: usize,
) -> (
    DocumentStore,
    DefinitionId,
    FeatureId,
    FeatureId,
    [(FeatureId, BooleanOperation); 4],
) {
    let definition = DefinitionId(80);
    let base_profile = FeatureId(800);
    let base = FeatureId(801);
    let tool_profile = FeatureId(802);
    let tool = FeatureId(803);
    let operations = [
        (FeatureId(804), BooleanOperation::Cut),
        (FeatureId(805), BooleanOperation::Union),
        (FeatureId(806), BooleanOperation::Intersect),
        (FeatureId(807), BooleanOperation::Split),
    ];
    let [scale_x, scale_y, scale_z] = scales;
    let scale_profile = |points: &[[f64; 2]]| {
        points
            .iter()
            .map(|[x, y]| [x * scale_x, y * scale_y])
            .collect::<Vec<_>>()
    };
    let base_points = rigid_profile_variant(
        &scale_profile(&[
            [-12.0, -8.0],
            [18.0, -6.0],
            [24.0, 9.0],
            [3.0, 20.0],
            [-17.0, 7.0],
        ]),
        rigid_variant,
    );
    let tool_points = rigid_profile_variant(
        &scale_profile(&[[-3.0, -15.0], [27.0, 4.0], [5.0, 24.0]]),
        rigid_variant,
    );
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: definition,
            name: "Generated Boolean property graph".into(),
        },
        CanonicalCommand::CreateFeature {
            id: base_profile,
            definition_id: definition,
            name: "Generated target profile".into(),
            kind: FeatureKind::Profile {
                points_mm: base_points,
            },
        },
        CanonicalCommand::CreateFeature {
            id: base,
            definition_id: definition,
            name: "Generated target body".into(),
            kind: FeatureKind::Extrusion {
                profile: base_profile,
                height: dimension(13.0 * scale_z),
            },
        },
        CanonicalCommand::CreateFeature {
            id: tool_profile,
            definition_id: definition,
            name: "Generated tool profile".into(),
            kind: FeatureKind::Profile {
                points_mm: tool_points,
            },
        },
        CanonicalCommand::CreateFeature {
            id: tool,
            definition_id: definition,
            name: "Generated tool body".into(),
            kind: FeatureKind::Extrusion {
                profile: tool_profile,
                height: dimension(19.0 * scale_z),
            },
        },
    ];
    commands.extend(
        operations.map(|(id, operation)| CanonicalCommand::CreateFeature {
            id,
            definition_id: definition,
            name: format!("Generated {operation:?}"),
            kind: FeatureKind::Boolean {
                operation,
                target: base,
                tool,
            },
        }),
    );
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    (document, definition, base, tool, operations)
}

#[test]
fn worker_binds_multiple_imported_sources_by_digest_for_boolean_and_mesh() {
    let (document, definition, base, tool, _) = generated_boolean_document([1.0, 1.0, 1.0], 0);
    let snapshot = document.current();
    let directory = tempfile::tempdir().unwrap();
    let cancelled = AtomicBool::new(false);
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let mut sources = Vec::new();
    let mut evidences = Vec::new();
    let mut source_volumes = Vec::new();
    for producer in [base, tool] {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        let path = directory.path().join(format!("source-{}.step", producer.0));
        supervisor
            .export_exact_brep_graph_step(&snapshot, &package, &path)
            .unwrap();
        let source = std::fs::read(&path).unwrap();
        let source_sha256 = sha256_hex(&source);
        let evidence = supervisor
            .inspect_step_import_with_cancellation(&path, &source_sha256, &cancelled)
            .unwrap();
        source_volumes.push(evidence.volume_mm3);
        sources.push(source);
        evidences.push(evidence);
    }

    let mut imported_document = DocumentStore::new();
    imported_document
        .apply_batch(
            &plan_step_import(
                &imported_document.current(),
                &sources[0],
                "target.step",
                &evidences[0],
            )
            .unwrap(),
        )
        .unwrap();
    let target_definition = imported_document
        .current()
        .definitions()
        .next()
        .unwrap()
        .id();
    let target = imported_document.current().features().next().unwrap().id();
    let target_occurrence = imported_document
        .current()
        .occurrences()
        .next()
        .unwrap()
        .id();
    let second_import = plan_step_import(
        &imported_document.current(),
        &sources[1],
        "tool.step",
        &evidences[1],
    )
    .unwrap();
    imported_document.apply_batch(&second_import).unwrap();
    let tool = imported_document
        .current()
        .features()
        .find(|feature| feature.id() != target)
        .unwrap()
        .id();
    let tool_occurrence = imported_document
        .current()
        .occurrences()
        .find(|occurrence| occurrence.id() != target_occurrence)
        .unwrap()
        .id();
    let mut container_data = persistence::ContainerData::default();
    for source in &sources {
        container_data.insert_import_blob(source.clone()).unwrap();
    }
    let imported_baseline =
        persistence::save_container(&imported_document.current(), &container_data).unwrap();
    let result_definition = DefinitionId(3);
    let result_features = [
        FeatureId(3),
        FeatureId(4),
        FeatureId(5),
        FeatureId(6),
        FeatureId(7),
    ];
    let angle = 30.0_f64.to_radians();
    let tool_transform = Transform::from_matrix([
        angle.cos(),
        -angle.sin(),
        0.0,
        4.0,
        angle.sin(),
        angle.cos(),
        0.0,
        -3.0,
        0.0,
        0.0,
        1.0,
        2.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap();
    let before_solid_tool = imported_document.current().canonical_digest();
    imported_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: tool_occurrence,
                transform: tool_transform,
            },
            CanonicalCommand::ApplySolidTool(SolidToolPlan {
                operation: BooleanOperation::Union,
                target_occurrence_id: target_occurrence,
                target_feature_id: target,
                tool_occurrence_id: tool_occurrence,
                tool_feature_id: tool,
                result_definition_id: result_definition,
                result_feature_ids: result_features.to_vec(),
                result_definition_name: "Imported source union".into(),
                result_feature_name: "Rigid imported union".into(),
                keep_tool: true,
            }),
        ]))
        .unwrap();
    let imported_snapshot = imported_document.current();
    assert_eq!(
        imported_snapshot
            .occurrence(target_occurrence)
            .unwrap()
            .definition_id(),
        result_definition
    );
    let producer = result_features[4];
    let graph =
        ExactBRepGraph::from_snapshot(&imported_snapshot, result_definition, producer).unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        &node.operation,
        ketchup_core::exact_brep_graph::ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if *matrix_bits == tool_transform.matrix().map(f64::to_bits)
    )));
    assert_ne!(target_definition, result_definition);
    let reversed_sources = [sources[1].as_slice(), sources[0].as_slice()];
    let package = supervisor
        .evaluate_exact_brep_graph_with_imported_sources(&graph, &reversed_sources)
        .unwrap();
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&graph, &reversed_sources)
            .unwrap(),
        package
    );
    assert_eq!(package.graph, graph);
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());
    assert!(package.volume_mm3 >= source_volumes[0].max(source_volumes[1]));
    assert!(package.volume_mm3 <= source_volumes.iter().sum::<f64>());
    assert!(package.is_current(&imported_snapshot));

    let mut imported_operation_packages = Vec::new();
    for operation in [BooleanOperation::Cut, BooleanOperation::Intersect] {
        let mut operation_document = persistence::load(&imported_baseline)
            .unwrap()
            .into_editable()
            .ok()
            .unwrap();
        operation_document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: tool_occurrence,
                    transform: tool_transform,
                },
                CanonicalCommand::ApplySolidTool(SolidToolPlan {
                    operation,
                    target_occurrence_id: target_occurrence,
                    target_feature_id: target,
                    tool_occurrence_id: tool_occurrence,
                    tool_feature_id: tool,
                    result_definition_id: result_definition,
                    result_feature_ids: result_features.to_vec(),
                    result_definition_name: format!("Imported source {operation:?}"),
                    result_feature_name: format!("Rigid imported {operation:?}"),
                    keep_tool: true,
                }),
            ]))
            .unwrap();
        let operation_snapshot = operation_document.current();
        let operation_graph = ExactBRepGraph::from_snapshot(
            &operation_snapshot,
            result_definition,
            result_features[4],
        )
        .unwrap();
        assert!(operation_graph.nodes.iter().any(|node| matches!(
            node.operation,
            ketchup_core::exact_brep_graph::ExactBRepOperation::Boolean {
                operation: graph_operation,
                ..
            } if graph_operation == operation.into()
        )));
        let operation_package = supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&operation_graph, &reversed_sources)
            .unwrap();
        assert_eq!(
            supervisor
                .evaluate_exact_brep_graph_with_imported_sources(
                    &operation_graph,
                    &reversed_sources,
                )
                .unwrap(),
            operation_package
        );
        assert!(operation_package.is_current(&operation_snapshot));
        imported_operation_packages.push(operation_package);
    }
    let cut_volume = imported_operation_packages[0].volume_mm3;
    let intersection_volume = imported_operation_packages[1].volume_mm3;
    assert!(cut_volume > 0.0);
    assert!(intersection_volume > 0.0);
    assert!(
        (cut_volume + intersection_volume - source_volumes[0]).abs() <= source_volumes[0] * 1.0e-8
    );
    let exact_body = ExactBodyPackage::Graph(package.clone());
    let result_key = exact_body.result_key();
    let registry = ExactResultRegistry::accept(&imported_snapshot, [Arc::new(exact_body)]).unwrap();
    assert!(registry.get_result(&result_key).is_some());
    let reopened = persistence::load(
        &persistence::save_container(&imported_snapshot, &container_data).unwrap(),
    )
    .unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert_eq!(
        reopened_snapshot.canonical_digest(),
        imported_snapshot.canonical_digest()
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened_snapshot, result_definition, producer).unwrap(),
        graph
    );
    let graph_step = directory.path().join("imported-union.step");
    supervisor
        .export_exact_brep_graph_step_with_imported_sources(
            &imported_snapshot,
            &package,
            &graph_step,
            &reversed_sources,
        )
        .unwrap();
    assert!(
        std::fs::read(&graph_step)
            .unwrap()
            .windows(9)
            .any(|window| window == b"ISO-10303")
    );
    let source_blobs = BTreeMap::from([
        (sha256_hex(&sources[0]), sources[0].clone()),
        (sha256_hex(&sources[1]), sources[1].clone()),
    ]);
    let model_step = directory.path().join("imported-union-model.step");
    supervisor
        .export_current_model_step_with_imported_sources(
            &imported_snapshot,
            &[(
                ExactBodyPackage::Graph(package.clone()),
                Transform::identity(),
            )],
            &model_step,
            &source_blobs,
        )
        .unwrap();
    assert!(
        std::fs::read(&model_step)
            .unwrap()
            .windows(9)
            .any(|window| window == b"ISO-10303")
    );
    assert!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&graph, &[sources[0].as_slice()])
            .is_err()
    );
    assert!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(
                &graph,
                &[sources[0].as_slice(), sources[0].as_slice()],
            )
            .is_err()
    );
    let malformed_source = b"not a STEP payload";
    let mut malformed_document = DocumentStore::new();
    malformed_document
        .apply_batch(
            &plan_step_import(
                &malformed_document.current(),
                malformed_source,
                "malformed.step",
                &evidences[0],
            )
            .unwrap(),
        )
        .unwrap();
    let malformed_snapshot = malformed_document.current();
    let malformed_definition = malformed_snapshot.definitions().next().unwrap().id();
    let malformed_producer = malformed_snapshot.features().next().unwrap().id();
    let malformed_graph = ExactBRepGraph::from_snapshot(
        &malformed_snapshot,
        malformed_definition,
        malformed_producer,
    )
    .unwrap();
    assert!(matches!(
        supervisor.evaluate_exact_brep_graph_with_imported_sources(
            &malformed_graph,
            &[malformed_source.as_slice()],
        ),
        Err(WorkerError::Geometry(_))
    ));
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph_with_imported_sources(&graph, &reversed_sources)
            .unwrap(),
        package
    );
    let undo_snapshot = imported_document.undo().unwrap();
    assert_eq!(undo_snapshot.canonical_digest(), before_solid_tool);
    assert_eq!(
        undo_snapshot
            .occurrence(target_occurrence)
            .unwrap()
            .definition_id(),
        target_definition
    );
    let redo_snapshot = imported_document.redo().unwrap();
    assert_eq!(
        redo_snapshot.canonical_digest(),
        imported_snapshot.canonical_digest()
    );
    assert!(package.is_current(&redo_snapshot));
}

#[test]
fn generated_boolean_graph_properties_cover_all_operations_and_rigid_variants() {
    let samples = generated_boolean_scales();
    assert_eq!(samples, generated_boolean_scales());
    assert_eq!(samples.len(), 6);
    for (index, sample) in samples.iter().enumerate() {
        assert!(
            sample
                .iter()
                .all(|value| value.is_finite() && *value >= 0.5 && *value <= 3.0)
        );
        assert!(samples.iter().skip(index + 1).all(|candidate| {
            candidate
                .iter()
                .zip(sample)
                .any(|(left, right)| left.to_bits() != right.to_bits())
        }));
    }

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for (sample_index, scales) in samples.into_iter().enumerate() {
        let mut variant_volumes = [[0.0; 4]; 3];
        for (rigid_variant, volumes) in variant_volumes.iter_mut().enumerate() {
            let (document, definition, base, tool, operations) =
                generated_boolean_document(scales, rigid_variant);
            let snapshot = document.current();
            let before_revision = snapshot.revision_id();
            let before_digest = snapshot.canonical_digest();
            let before_undo = document.visible_undo_steps();
            let base_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, base).unwrap();
            let tool_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, tool).unwrap();
            let base_result = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
            let tool_result = supervisor.evaluate_exact_brep_graph(&tool_graph).unwrap();
            let mut packages = Vec::new();

            for (operation_index, (producer, operation)) in operations.into_iter().enumerate() {
                let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
                let package = supervisor
                    .evaluate_exact_brep_graph(&graph)
                    .unwrap_or_else(|error| {
                        panic!(
                            "sample {sample_index}, rigid variant {rigid_variant}, {operation:?}: {error:?}"
                        )
                    });
                assert_eq!(
                    supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
                    package,
                    "sample {sample_index}, rigid variant {rigid_variant}, {operation:?} is not deterministic"
                );
                assert_eq!(package.graph, graph);
                assert_eq!(
                    package.identity.canonical_input_digest,
                    graph.canonical_input_digest
                );
                assert_eq!(package.identity.producer_feature_id.0, producer.0);
                assert!(package.is_current(&snapshot));
                volumes[operation_index] = package.volume_mm3;
                packages.push(package);
            }

            let tolerance = base_result.volume_mm3.max(tool_result.volume_mm3) * 1.0e-9;
            assert!(
                (volumes[0] + volumes[2] - base_result.volume_mm3).abs() <= tolerance,
                "sample {sample_index}, rigid variant {rigid_variant}: cut + intersection must equal target"
            );
            assert!(
                (volumes[1] + volumes[2] - base_result.volume_mm3 - tool_result.volume_mm3).abs()
                    <= tolerance,
                "sample {sample_index}, rigid variant {rigid_variant}: union + intersection must equal target + tool"
            );
            assert!(
                (volumes[3] - base_result.volume_mm3).abs() <= tolerance,
                "sample {sample_index}, rigid variant {rigid_variant}: split must preserve target volume"
            );
            assert!(packages[3].topology_counts[4] >= 2);

            let registry = ExactResultRegistry::accept(
                &snapshot,
                packages
                    .iter()
                    .cloned()
                    .map(ExactBodyPackage::Graph)
                    .map(Arc::new),
            )
            .unwrap();
            for package in &packages {
                assert!(
                    registry
                        .get_result(&ExactBodyPackage::Graph(package.clone()).result_key())
                        .is_some()
                );
            }
            assert_eq!(document.current().revision_id(), before_revision);
            assert_eq!(document.current().canonical_digest(), before_digest);
            assert_eq!(document.visible_undo_steps(), before_undo);

            let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
            let reopened_snapshot = reopened.snapshot();
            assert_eq!(reopened_snapshot.canonical_digest(), before_digest);
            for ((producer, _), package) in operations.into_iter().zip(packages) {
                assert_eq!(
                    ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, producer)
                        .unwrap(),
                    package.graph
                );
                assert!(package.is_current(&reopened_snapshot));
            }
        }

        for operation_index in 0..4 {
            let base = variant_volumes[0][operation_index];
            for (rigid_variant, transformed) in variant_volumes.iter().enumerate().skip(1) {
                assert!(
                    (transformed[operation_index] - base).abs() <= base.max(1.0) * 1.0e-9,
                    "sample {sample_index}, operation {operation_index}: rigid variant {rigid_variant} changed volume"
                );
            }
        }
    }
}

#[test]
fn generated_boolean_graph_property_verifier_confirms_round_trip_and_scaling() {
    let samples = generated_boolean_scales();
    assert_eq!(samples, generated_boolean_scales());
    assert_eq!(samples.len(), 6);
    assert_eq!(samples[0], [0.5, 0.75, 0.6]);
    assert_eq!(samples[2], [3.0, 2.5, 1.75]);
    for (index, [scale_x, scale_y, scale_z]) in samples.iter().copied().enumerate() {
        assert!((0.5..=3.0).contains(&scale_x));
        assert!((0.5..=2.5).contains(&scale_y));
        assert!((0.5..=1.75).contains(&scale_z));
        assert!(samples.iter().skip(index + 1).all(|candidate| {
            candidate
                .iter()
                .zip([scale_x, scale_y, scale_z])
                .any(|(left, right)| left.to_bits() != right.to_bits())
        }));
    }

    let verification_samples = [samples[0], samples[2], samples[5]];
    let mut normalized_reference: Option<[f64; 4]> = None;
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    for (sample_index, scales) in verification_samples.into_iter().enumerate() {
        let mut rigid_reference: Option<[f64; 4]> = None;
        for rigid_variant in 0..3 {
            let (document, definition, base, tool, operations) =
                generated_boolean_document(scales, rigid_variant);
            let snapshot = document.current();
            let before_revision = snapshot.revision_id();
            let before_digest = snapshot.canonical_digest();
            let before_undo = document.visible_undo_steps();
            let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
            let reopened_snapshot = reopened.snapshot();
            assert_eq!(reopened_snapshot.canonical_digest(), before_digest);

            let base_result = supervisor
                .evaluate_exact_brep_graph(
                    &ExactBRepGraph::from_snapshot(&snapshot, definition, base).unwrap(),
                )
                .unwrap();
            let tool_result = supervisor
                .evaluate_exact_brep_graph(
                    &ExactBRepGraph::from_snapshot(&snapshot, definition, tool).unwrap(),
                )
                .unwrap();
            let mut volumes = [0.0; 4];
            for (operation_index, (producer, operation)) in operations.into_iter().enumerate() {
                let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
                let reopened_graph =
                    ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, producer)
                        .unwrap();
                assert_eq!(reopened_graph, graph);
                let package = supervisor
                    .evaluate_exact_brep_graph(&graph)
                    .unwrap_or_else(|error| {
                        panic!(
                            "verifier sample {sample_index}, rigid variant {rigid_variant}, {operation:?}: {error:?}"
                        )
                    });
                assert_eq!(
                    supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
                    package
                );
                assert_eq!(
                    supervisor
                        .evaluate_exact_brep_graph(&reopened_graph)
                        .unwrap(),
                    package
                );
                assert!(package.is_current(&snapshot));
                assert!(package.is_current(&reopened_snapshot));
                volumes[operation_index] = package.volume_mm3;
            }

            let tolerance = base_result.volume_mm3.max(tool_result.volume_mm3) * 1.0e-9;
            assert!((volumes[0] + volumes[2] - base_result.volume_mm3).abs() <= tolerance);
            assert!(
                (volumes[1] + volumes[2] - base_result.volume_mm3 - tool_result.volume_mm3).abs()
                    <= tolerance
            );
            assert!((volumes[3] - base_result.volume_mm3).abs() <= tolerance);
            if let Some(reference) = rigid_reference {
                for (actual, expected) in volumes.into_iter().zip(reference) {
                    assert!((actual - expected).abs() <= expected.max(1.0) * 1.0e-9);
                }
            } else {
                rigid_reference = Some(volumes);
            }

            assert_eq!(document.current().revision_id(), before_revision);
            assert_eq!(document.current().canonical_digest(), before_digest);
            assert_eq!(document.visible_undo_steps(), before_undo);
        }

        let scale_product = scales.into_iter().product::<f64>();
        let normalized = rigid_reference
            .unwrap()
            .map(|volume| volume / scale_product);
        if let Some(reference) = normalized_reference {
            for (actual, expected) in normalized.into_iter().zip(reference) {
                assert!((actual - expected).abs() <= expected.max(1.0) * 1.0e-9);
            }
        } else {
            normalized_reference = Some(normalized);
        }
    }
}

#[test]
fn generated_boolean_graph_preserves_legacy_export_and_stale_contracts() {
    let (mut document, definition, base, _, operations) =
        generated_boolean_document([1.0, 1.0, 1.0], 0);
    let intersect = operations[2].0;
    let split = operations[3].0;
    let snapshot = document.current();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot_for_producer(&snapshot, definition, intersect),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );

    let split_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, split).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let split_package = supervisor.evaluate_exact_brep_graph(&split_graph).unwrap();
    let split_body = ExactBodyPackage::Graph(split_package.clone());
    let result_key = split_body.result_key();
    let registry = ExactResultRegistry::accept(&snapshot, [Arc::new(split_body)]).unwrap();
    assert!(registry.get_result(&result_key).is_some());

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    assert!(split_package.is_current(&reopened_snapshot));
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, split).unwrap(),
        split_package.graph
    );
    ExactResultRegistry::accept(
        &reopened_snapshot,
        [Arc::new(ExactBodyPackage::Graph(split_package.clone()))],
    )
    .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("generated-boolean-graph.step");
    supervisor
        .export_exact_brep_graph_step(&snapshot, &split_package, &step_path)
        .unwrap();
    let step = std::fs::read(&step_path).unwrap();
    assert!(step.len() > 256);
    assert!(step.windows(9).any(|window| window == b"ISO-10303"));
    let model_step_path = directory.path().join("generated-boolean-model.step");
    supervisor
        .export_current_model_step(
            &snapshot,
            &[(
                ExactBodyPackage::Graph(split_package.clone()),
                Transform::identity(),
            )],
            &model_step_path,
        )
        .unwrap();
    let model_step = std::fs::read(&model_step_path).unwrap();
    assert!(model_step.len() > 256);
    assert!(model_step.windows(9).any(|window| window == b"ISO-10303"));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: base,
                dimension: dimension(17.0),
            },
        ]))
        .unwrap();
    let edited_snapshot = document.current();
    assert!(!split_package.is_current(&edited_snapshot));
    assert!(matches!(
        ExactResultRegistry::accept(
            &edited_snapshot,
            [Arc::new(ExactBodyPackage::Graph(split_package.clone()))]
        ),
        Err(ExactProductError::StaleResult)
    ));
    assert!(
        supervisor
            .export_exact_brep_graph_step(&edited_snapshot, &split_package, &step_path)
            .is_err()
    );
    let edited_graph = ExactBRepGraph::from_snapshot(&edited_snapshot, definition, split).unwrap();
    assert_ne!(edited_graph.graph_digest, split_package.graph.graph_digest);
    let edited_package = supervisor.evaluate_exact_brep_graph(&edited_graph).unwrap();
    ExactResultRegistry::accept(
        &edited_snapshot,
        [Arc::new(ExactBodyPackage::Graph(edited_package.clone()))],
    )
    .unwrap();

    let undo_snapshot = document.undo().unwrap();
    assert_eq!(
        undo_snapshot.canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(split_package.is_current(&undo_snapshot));
    assert!(!edited_package.is_current(&undo_snapshot));
    let redo_snapshot = document.redo().unwrap();
    assert_eq!(
        redo_snapshot.canonical_digest(),
        edited_snapshot.canonical_digest()
    );
    assert!(edited_package.is_current(&redo_snapshot));
    assert!(!split_package.is_current(&redo_snapshot));
}

#[test]
fn worker_transforms_a_circle_pad_from_its_arbitrary_workplane_frame() {
    let definition = DefinitionId(2);
    let plane = FeatureId(100);
    let sketch_id = FeatureId(101);
    let pad = FeatureId(102);
    let sketch = SketchSpec {
        workplane: plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [10.0, 20.0],
            radius_mm: 5.0,
        }],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Oriented Pad graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Circle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(25.0)),
                }),
            },
        ]))
        .unwrap();
    let graph = ExactBRepGraph::from_snapshot(&document.current(), definition, pad).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let mesh = StepImportMesh {
        vertices_mm: result
            .vertices
            .iter()
            .map(|vertex| vertex.position_mm)
            .collect(),
        triangles: result
            .triangles
            .iter()
            .zip(&result.triangle_face_ordinals)
            .map(|(triangle, face_ordinal)| StepMeshTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: *face_ordinal,
            })
            .collect(),
    };
    for invalid_area in [1.0, f64::NAN] {
        assert!(matches!(
            ExactBRepGraphPackage::from_worker_evidence(
                &graph,
                ExactBRepGraphWorkerEvidence {
                    exact_input_digest: result.identity.exact_input_digest.clone(),
                    result_fingerprint: result.identity.result_fingerprint.clone(),
                    volume_mm3: result.volume_mm3,
                    area_mm2: invalid_area,
                    topology_counts: result.topology_counts,
                    bounds_mm: result.bounds_mm,
                    backend: result.identity.backend.clone(),
                    tolerance: result.identity.tolerance.clone(),
                },
                &mesh,
            ),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
    }

    assert_bounds_close(result.bounds_mm, [0.0, 5.0, 15.0, 25.0, 15.0, 25.0]);
    assert_eq!(result.identity.producer_feature_id.0, pad.0);
    assert_eq!(result.topology_counts[4], 1);
    assert_eq!(
        result.topological_references.len(),
        result.topology_counts[..3]
            .iter()
            .map(|count| *count as usize)
            .sum::<usize>()
    );
    for (kind, count) in [
        (TopologicalElementKind::Vertex, result.topology_counts[0]),
        (TopologicalElementKind::Edge, result.topology_counts[1]),
        (TopologicalElementKind::Face, result.topology_counts[2]),
    ] {
        assert_eq!(
            result
                .topological_references
                .iter()
                .filter(|reference| reference.kind == kind)
                .count(),
            count as usize
        );
    }
    assert!(result.topological_references.iter().all(|reference| {
        reference.has_valid_lineage()
            && reference.producer_feature_id == pad
            && reference.stability == TopologicalReferenceStability::Ephemeral
    }));
}

#[test]
fn worker_evaluates_a_bounded_profile_pocket_as_a_body_cut() {
    let definition = DefinitionId(3);
    let base_profile = FeatureId(200);
    let base = FeatureId(201);
    let pocket_profile = FeatureId(202);
    let pocket = FeatureId(203);
    let through_cut = FeatureId(204);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Pocket graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Base boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(10.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: pocket_profile,
                definition_id: definition,
                name: "Triangular pocket".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[6.0, 5.0], [24.0, 7.0], [12.0, 16.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: pocket,
                definition_id: definition,
                name: "Pocket".into(),
                kind: FeatureKind::Pocket {
                    target: base,
                    profile: pocket_profile,
                    depth: dimension(4.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: through_cut,
                definition_id: definition,
                name: "Through cut".into(),
                kind: FeatureKind::ThroughCut {
                    target: base,
                    profile: pocket_profile,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, pocket).unwrap();
    let through_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, through_cut).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let through_result = supervisor
        .evaluate_exact_brep_graph(&through_graph)
        .unwrap();

    assert!(result.volume_mm3 > 0.0 && result.volume_mm3 < 6_000.0);
    assert_bounds_close(result.bounds_mm, [0.0, 0.0, 0.0, 30.0, 20.0, 10.0]);
    assert_eq!(result.identity.producer_feature_id.0, pocket.0);
    assert!(through_result.volume_mm3 > 0.0 && through_result.volume_mm3 < result.volume_mm3);
    assert_bounds_close(through_result.bounds_mm, [0.0, 0.0, 0.0, 30.0, 20.0, 10.0]);
    assert_ne!(
        through_result.identity.result_fingerprint,
        result.identity.result_fingerprint
    );
}

#[test]
fn worker_booleans_an_unequal_body_transformed_by_its_workplane_frame() {
    let definition = DefinitionId(4);
    let base_profile = FeatureId(300);
    let base = FeatureId(301);
    let plane = FeatureId(302);
    let sketch_id = FeatureId(303);
    let tool = FeatureId(304);
    let intersection = FeatureId(305);
    let sketch = SketchSpec {
        workplane: plane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 5.0],
            radius_mm: 4.0,
        }],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Transformed Boolean graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Rectangular base".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, -10.0], [30.0, -10.0], [30.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(10.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: plane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Transverse circle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: tool,
                definition_id: definition,
                name: "Transverse tool".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(25.0)),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: intersection,
                definition_id: definition,
                name: "Transformed intersection".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Intersect,
                    target: base,
                    tool,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let tool_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, tool).unwrap();
    let intersection_graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition, intersection).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let tool_result = supervisor.evaluate_exact_brep_graph(&tool_graph).unwrap();
    let intersection_result = supervisor
        .evaluate_exact_brep_graph(&intersection_graph)
        .unwrap();

    assert_eq!(intersection_result.volume_mm3, tool_result.volume_mm3);
    assert_bounds_close(
        intersection_result.bounds_mm,
        [0.0, -4.0, 1.0, 25.0, 4.0, 9.0],
    );
    assert_eq!(
        intersection_result.identity.producer_feature_id.0,
        intersection.0
    );
}

#[test]
fn disjoint_body_booleans_return_exact_results_or_typed_refusals_atomically() {
    let definition = DefinitionId(5);
    let base_profile = FeatureId(400);
    let base = FeatureId(401);
    let tool_profile = FeatureId(402);
    let tool = FeatureId(403);
    let cut = FeatureId(404);
    let union = FeatureId(405);
    let intersect = FeatureId(406);
    let split = FeatureId(407);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Disjoint Boolean graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_profile,
                definition_id: definition,
                name: "Base profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Extrusion {
                    profile: base_profile,
                    height: dimension(5.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: tool_profile,
                definition_id: definition,
                name: "Remote tool profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[30.0, 0.0], [35.0, 0.0], [35.0, 5.0], [30.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: tool,
                definition_id: definition,
                name: "Remote tool".into(),
                kind: FeatureKind::Extrusion {
                    profile: tool_profile,
                    height: dimension(7.0),
                },
            },
        ]))
        .unwrap();
    for (id, operation) in [
        (cut, BooleanOperation::Cut),
        (union, BooleanOperation::Union),
        (intersect, BooleanOperation::Intersect),
        (split, BooleanOperation::Split),
    ] {
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id: definition,
                name: format!("{operation:?}"),
                kind: FeatureKind::Boolean {
                    operation,
                    target: base,
                    tool,
                },
            }]))
            .unwrap();
    }
    let snapshot = document.current();
    let graph = |producer| ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
    let base_graph = graph(base);
    let tool_graph = graph(tool);
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_result = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let tool_result = supervisor.evaluate_exact_brep_graph(&tool_graph).unwrap();
    let cut_result = supervisor.evaluate_exact_brep_graph(&graph(cut)).unwrap();
    let union_result = supervisor.evaluate_exact_brep_graph(&graph(union)).unwrap();

    assert!((cut_result.volume_mm3 - base_result.volume_mm3).abs() <= 1.0e-6);
    assert!(
        (union_result.volume_mm3 - base_result.volume_mm3 - tool_result.volume_mm3).abs() <= 1.0e-6
    );
    assert_eq!(union_result.topology_counts[4], 2);

    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&graph(intersect))
            .unwrap_err(),
        "invalid_shape",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_result
    );
    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&graph(split))
            .unwrap_err(),
        "no_geometric_change",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_result
    );
}

#[test]
fn graph_results_are_stale_safe_and_resource_or_unsupported_inputs_fail_closed() {
    let valid_graph = simple_extrusion_graph();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let valid_result = supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap();

    let node = NodeId(77);
    let mut scheduler = EvaluationScheduler::new(1_024);
    scheduler
        .advance_revision(valid_graph.source_revision, [node])
        .unwrap();
    let stale_token = scheduler
        .schedule(node, valid_graph.canonical_input_digest.clone())
        .unwrap();
    scheduler
        .advance_revision(valid_graph.source_revision + 1, [node])
        .unwrap();
    assert_eq!(
        scheduler.accept(DerivedResult {
            token: stale_token,
            result_fingerprint: valid_result.identity.result_fingerprint.clone(),
            charge_bytes: 64,
        }),
        InsertOutcome::Stale
    );
    assert_eq!(scheduler.current_result_fingerprint(node), None);

    let mut oversized = valid_graph.clone();
    let profile = oversized.profiles[0].clone();
    oversized
        .profiles
        .resize(MAX_EXACT_BREP_GRAPH_PROFILES + 1, profile);
    match supervisor
        .evaluate_exact_brep_graph(&oversized)
        .unwrap_err()
    {
        WorkerError::Protocol(detail) => {
            assert!(detail.contains("resource limit") || detail.contains("structure is invalid"));
        }
        other => panic!("expected bounded graph refusal, got {other:?}"),
    }
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap(),
        valid_result
    );

    let definition = DefinitionId(91);
    let profile = FeatureId(910);
    let path = FeatureId(911);
    let sweep = FeatureId(912);
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Unsupported operation graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Spline sweep profile".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-3.0, -2.0], [4.0, -2.0], [4.0, 3.0], [-3.0, 3.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Straight path".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [20.0, 0.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Unsupported spline sweep".into(),
                kind: FeatureKind::Sweep { profile, path },
            },
        ]))
        .err()
        .expect("spline Sweep must fail before exact worker evaluation");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&valid_graph).unwrap(),
        valid_result
    );
}

#[test]
fn worker_evaluates_circle_sketch_revolve_in_its_workplane_frame() {
    let definition = DefinitionId(60);
    let workplane = FeatureId(600);
    let sketch = FeatureId(601);
    let revolve = FeatureId(602);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Framed sketch revolve".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch,
                definition_id: definition,
                name: "Circle sketch".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [10.0, 0.0],
                        radius_mm: 2.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Framed revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: sketch,
                    axis_start_mm: [0.0, 0.0],
                    axis_end_mm: [0.0, 1.0],
                    angle_degrees: 360.0,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, revolve).unwrap();
    assert_eq!(graph.profiles[0].source_feature_id, sketch.0);
    assert!(graph.profiles[0].region_id.is_some());

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id.0, revolve.0);
    assert!(package.volume_mm3 > 0.0);
    assert_bounds_close(package.bounds_mm, [-12.0, -12.0, -2.0, 12.0, 12.0, 2.0]);
}

#[test]
fn worker_evaluates_compound_mixed_sketch_revolve_with_stable_persistence_and_undo() {
    let definition = DefinitionId(61);
    let workplane = FeatureId(610);
    let sketch_id = FeatureId(611);
    let revolve = FeatureId(612);
    let sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [10.0, -10.0],
                end_mm: [30.0, -10.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [30.0, -10.0],
                end_mm: [30.0, 10.0],
            },
            SketchEntity::CubicBezier {
                id: SketchEntityId(3),
                start_mm: [30.0, 10.0],
                control_1_mm: [26.0, 14.0],
                control_2_mm: [22.0, 14.0],
                end_mm: [18.0, 10.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(4),
                start_mm: [18.0, 10.0],
                end_mm: [10.0, 2.0],
                center_mm: [18.0, 2.0],
                clockwise: false,
            },
            SketchEntity::Line {
                id: SketchEntityId(5),
                start_mm: [10.0, 2.0],
                end_mm: [10.0, -10.0],
            },
            SketchEntity::Circle {
                id: SketchEntityId(6),
                center_mm: [20.0, 0.0],
                radius_mm: 2.0,
            },
        ],
        constraints: Vec::new(),
    };
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].holes.len(), 1);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Compound mixed revolve".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Mixed outer loop and circle hole".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Bounded compound revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: sketch_id,
                    axis_start_mm: [0.0, -20.0],
                    axis_end_mm: [0.0, 20.0],
                    angle_degrees: 270.0,
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, revolve).unwrap();
    let ExactBRepPlanarGeometry::Region { outer, holes } = &graph.profiles[0].geometry else {
        panic!("compound sketch must compile to a planar region");
    };
    let ExactBRepPlanarLoop::Boundary { segments } = outer else {
        panic!("mixed outer loop must remain a boundary");
    };
    assert_eq!(segments.len(), 5);
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
    assert_eq!(holes.len(), 1);
    assert!(matches!(holes[0], ExactBRepPlanarLoop::Circle { .. }));

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id.0, revolve.0);
    assert_eq!(package.topology_counts[4], 1);
    assert!(package.volume_mm3 > 0.0);

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, revolve).unwrap();
    assert_eq!(reopened_graph, graph);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&reopened_graph)
            .unwrap(),
        package
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: revolve },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Bounded compound revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: sketch_id,
                    axis_start_mm: [0.0, -20.0],
                    axis_end_mm: [0.0, 20.0],
                    angle_degrees: 180.0,
                },
            },
        ]))
        .unwrap();
    let changed = ExactBRepGraph::from_snapshot(&document.current(), definition, revolve).unwrap();
    assert_ne!(changed.graph_digest, graph.graph_digest);
    assert!(!package.is_current(&document.current()));
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.undo().unwrap(), definition, revolve).unwrap(),
        graph
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.redo().unwrap(), definition, revolve).unwrap(),
        changed
    );
}

#[test]
fn worker_evaluates_revolve_non_rectangular_sweep_and_loft_through_one_graph_identity() {
    let definition = DefinitionId(6);
    let revolve_profile = FeatureId(500);
    let revolve = FeatureId(501);
    let sweep_profile = FeatureId(502);
    let sweep_path = FeatureId(503);
    let sweep = FeatureId(504);
    let loft_lower = FeatureId(505);
    let loft_upper = FeatureId(506);
    let loft = FeatureId(507);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Unified exact graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: revolve_profile,
                definition_id: definition,
                name: "Asymmetric revolve boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, -4.0], [7.0, -3.0], [5.0, 6.0], [2.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: definition,
                name: "Partial revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: revolve_profile,
                    axis_start_mm: [0.0, -10.0],
                    axis_end_mm: [0.0, 10.0],
                    angle_degrees: 270.0,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep_profile,
                definition_id: definition,
                name: "Curved sweep boundary".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [-2.0, -3.0],
                            end_mm: [2.0, -3.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [2.0, -3.0],
                            end_mm: [2.0, 3.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [2.0, 3.0],
                            end_mm: [-2.0, 3.0],
                            center_mm: [0.0, 3.0],
                            clockwise: false,
                        },
                        ProfileSegment::Line {
                            start_mm: [-2.0, 3.0],
                            end_mm: [-2.0, -3.0],
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep_path,
                definition_id: definition,
                name: "Oblique sweep path".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [10.0, -5.0],
                        end_mm: [24.0, 17.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: definition,
                name: "Non-rectangular sweep".into(),
                kind: FeatureKind::Sweep {
                    profile: sweep_profile,
                    path: sweep_path,
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft_lower,
                definition_id: definition,
                name: "Lower spline".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -5.0], [9.0, -4.0], [8.0, 6.0], [-7.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft_upper,
                definition_id: definition,
                name: "Upper spline".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-4.0, -3.0], [6.0, -2.0], [5.0, 4.0], [-3.0, 3.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: definition,
                name: "Bounded loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: loft_lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: loft_upper,
                            elevation_mm: 18.0,
                        },
                    ],
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();

    for producer in [revolve, sweep, loft] {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
        let first = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        let repeated = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(first, repeated);
        assert_eq!(
            first.identity.canonical_input_digest,
            graph.canonical_input_digest
        );
        assert_eq!(first.graph.graph_digest, graph.graph_digest);
        assert_eq!(first.identity.producer_feature_id.0, producer.0);
        assert!(!first.identity.result_fingerprint.is_empty());
        assert!(first.volume_mm3 > 0.0);
        assert_eq!(first.topology_counts[4], 1);
    }
}

#[test]
fn worker_evaluates_planar_offset_face_through_exact_brep_graph() {
    let definition = DefinitionId(96);
    let profile = FeatureId(960);
    let offset = FeatureId(961);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Graph planar offset".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Line-arc capsule".into(),
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
                id: offset,
                definition_id: definition,
                name: "Offset face".into(),
                kind: FeatureKind::PlanarOffset {
                    profile,
                    distance: dimension(2.0),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, offset).unwrap();
    assert!(graph.terminal_is_planar_offset());

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let request = ExactPlanarOffsetRequest::from_snapshot(&snapshot, definition).unwrap();
    let dedicated = supervisor.evaluate_planar_offset(&request).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    let mesh = StepImportMesh {
        vertices_mm: package
            .vertices
            .iter()
            .map(|vertex| vertex.position_mm)
            .collect(),
        triangles: package
            .triangles
            .iter()
            .zip(&package.triangle_face_ordinals)
            .map(|(triangle, face_ordinal)| StepMeshTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: *face_ordinal,
            })
            .collect(),
    };
    let evidence = |area_mm2| ExactBRepGraphWorkerEvidence {
        exact_input_digest: package.identity.exact_input_digest.clone(),
        result_fingerprint: package.identity.result_fingerprint.clone(),
        volume_mm3: package.volume_mm3,
        area_mm2,
        topology_counts: package.topology_counts,
        bounds_mm: package.bounds_mm,
        backend: package.identity.backend.clone(),
        tolerance: package.identity.tolerance.clone(),
    };
    assert!(
        ExactBRepGraphPackage::from_worker_evidence(&graph, evidence(package.area_mm2), &mesh,)
            .is_ok()
    );
    for forged_area in [package.area_mm2 * 0.5, f64::NAN] {
        assert!(matches!(
            ExactBRepGraphPackage::from_worker_evidence(&graph, evidence(forged_area), &mesh,),
            Err(ExactProductError::InvalidWorkerEvidence)
        ));
    }
    assert!(package.is_current(&snapshot));
    assert_eq!(package.volume_mm3, 0.0);
    assert!(package.topology_counts[0] > 0);
    assert_eq!(package.topology_counts[0], package.topology_counts[1]);
    assert_eq!(package.topology_counts[2..], [1, 0, 0]);
    assert_eq!(package.bounds_mm[0][2], 0.0);
    assert_eq!(package.bounds_mm[1][2], 0.0);
    assert!(!package.vertices.is_empty());
    assert!(!package.triangles.is_empty());
    assert_eq!(
        package.triangles.len(),
        package.triangle_face_ordinals.len()
    );
    assert!(
        package
            .triangle_face_ordinals
            .iter()
            .all(|ordinal| *ordinal == 0)
    );
    assert_eq!(
        package.identity.result_fingerprint,
        dedicated.identity.result_fingerprint
    );
    assert_eq!(package.bounds_mm, dedicated.bounds_mm);
}

#[test]
fn worker_evaluates_signed_linear_intervals_through_one_graph_protocol() {
    let definition = DefinitionId(7);
    let workplane = FeatureId(600);
    let sketch_id = FeatureId(601);
    let one_sided = FeatureId(602);
    let symmetric = FeatureId(603);
    let bidirectional = FeatureId(604);
    let sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-2.0, -3.0],
                end_mm: [2.0, -3.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [2.0, -3.0],
                end_mm: [2.0, 3.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [2.0, 3.0],
                end_mm: [-2.0, 3.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [-2.0, 3.0],
                end_mm: [-2.0, -3.0],
            },
        ],
        constraints: Vec::new(),
    };
    let region = sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Signed interval graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Shared rectangle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: one_sided,
                definition_id: definition,
                name: "One-sided".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: symmetric,
                definition_id: definition,
                name: "Symmetric oblique".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::Vector([1.0, 0.0, 1.0]),
                    extent: FeatureExtent::Symmetric(dimension(10.0)),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: bidirectional,
                definition_id: definition,
                name: "Unequal bidirectional".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Bidirectional {
                        along: FeatureExtentEnd::Blind(dimension(7.0)),
                        opposite: FeatureExtentEnd::Blind(dimension(3.0)),
                    },
                }),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let mut packages = Vec::new();
    for producer in [one_sided, symmetric, bidirectional] {
        let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, producer).unwrap();
        let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
        assert_eq!(
            supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
            package
        );
        assert_eq!(
            package.identity.canonical_input_digest,
            graph.canonical_input_digest
        );
        assert_eq!(package.graph.graph_digest, graph.graph_digest);
        packages.push(package);
    }

    assert_bounds_close(packages[0].bounds_mm, [-2.0, -3.0, 0.0, 2.0, 3.0, 10.0]);
    let oblique = 5.0 / 2.0_f64.sqrt();
    assert_bounds_close(
        packages[1].bounds_mm,
        [-2.0 - oblique, -3.0, -oblique, 2.0 + oblique, 3.0, oblique],
    );
    assert_bounds_close(packages[2].bounds_mm, [-2.0, -3.0, -3.0, 2.0, 3.0, 7.0]);
    assert_ne!(
        packages[0].identity.result_fingerprint,
        packages[1].identity.result_fingerprint
    );
    assert_ne!(
        packages[0].identity.result_fingerprint,
        packages[2].identity.result_fingerprint
    );
    assert_ne!(
        packages[1].identity.result_fingerprint,
        packages[2].identity.result_fingerprint
    );

    let registry = ExactResultRegistry::accept(
        &snapshot,
        packages
            .iter()
            .cloned()
            .map(ExactBodyPackage::Graph)
            .map(Arc::new),
    )
    .unwrap();
    for package in &packages {
        assert!(
            registry
                .get_result(&ExactBodyPackage::Graph(package.clone()).result_key())
                .is_some()
        );
    }

    let directory = tempfile::tempdir().unwrap();
    for package in &packages {
        let path = directory.path().join(format!(
            "signed-{}.step",
            package.identity.producer_feature_id.0
        ));
        supervisor
            .export_exact_brep_graph_step(&snapshot, package, &path)
            .unwrap();
        let step = std::fs::read(path).unwrap();
        assert!(step.len() > 256);
        assert!(step.windows(9).any(|window| window == b"ISO-10303"));
    }
}

#[test]
fn worker_applies_topology_selected_shell_fillet_and_chamfer_and_rejects_stale_identity() {
    let definition = DefinitionId(8);
    let profile = FeatureId(700);
    let base = FeatureId(701);
    let shell = FeatureId(702);
    let fillet = FeatureId(703);
    let chamfer = FeatureId(704);
    let stale_finish = FeatureId(705);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Topology-selected finishes".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Unequal rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [37.0, 0.0], [37.0, 23.0], [0.0, 23.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Exact base".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: dimension(19.0),
                },
            },
        ]))
        .unwrap();

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_graph = ExactBRepGraph::from_snapshot(&document.current(), definition, base).unwrap();
    let base_package = supervisor.evaluate_exact_brep_graph(&base_graph).unwrap();
    let top_z = base_package.bounds_mm[1][2];
    let top_face_ordinal = base_package
        .triangles
        .iter()
        .zip(&base_package.triangle_face_ordinals)
        .find_map(|(triangle, face_ordinal)| {
            triangle
                .vertex_indices
                .iter()
                .all(|index| {
                    (base_package.vertices[*index as usize].position_mm[2] - top_z).abs() <= 1.0e-6
                })
                .then_some(*face_ordinal)
        })
        .unwrap();
    let face = base_package
        .topological_references
        .iter()
        .find(|reference| {
            reference.kind == TopologicalElementKind::Face
                && reference.producer_element_id
                    == format!("generated-result/face/{top_face_ordinal}")
        })
        .unwrap()
        .clone();
    let edge = base_package
        .topological_references
        .iter()
        .find(|reference| reference.kind == TopologicalElementKind::Edge)
        .unwrap()
        .clone();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: shell,
                definition_id: definition,
                name: "Selected-face shell".into(),
                kind: FeatureKind::TopologyShell {
                    target: base,
                    removed_faces: vec![face],
                    thickness: dimension(1.5),
                },
            },
            CanonicalCommand::CreateFeature {
                id: fillet,
                definition_id: definition,
                name: "Selected-edge fillet".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: vec![edge.clone()],
                    kind: EdgeFinishKind::Fillet,
                    amount: dimension(0.75),
                },
            },
            CanonicalCommand::CreateFeature {
                id: chamfer,
                definition_id: definition,
                name: "Selected-edge chamfer".into(),
                kind: FeatureKind::TopologyEdgeFinish {
                    target: base,
                    edges: vec![edge.clone()],
                    kind: EdgeFinishKind::Chamfer,
                    amount: dimension(0.75),
                },
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let shell_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, shell).unwrap();
    let fillet_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, fillet).unwrap();
    let chamfer_graph = ExactBRepGraph::from_snapshot(&snapshot, definition, chamfer).unwrap();
    let shell_package = supervisor.evaluate_exact_brep_graph(&shell_graph).unwrap();
    let fillet_package = supervisor.evaluate_exact_brep_graph(&fillet_graph).unwrap();
    let chamfer_package = supervisor
        .evaluate_exact_brep_graph(&chamfer_graph)
        .unwrap();

    assert!(shell_package.volume_mm3 < base_package.volume_mm3);
    assert_eq!(shell_package.topology_counts[4], 1);
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&shell_graph).unwrap(),
        shell_package
    );
    assert_eq!(fillet_package.topology_counts[4], 1);
    assert_eq!(chamfer_package.topology_counts[4], 1);
    assert_ne!(
        fillet_package.identity.result_fingerprint,
        chamfer_package.identity.result_fingerprint
    );

    let stale_edge = TopologicalElementRef::new(
        edge.document_id,
        edge.definition_id,
        edge.source_feature_id,
        edge.producer_feature_id,
        edge.kind,
        edge.source_element_id,
        edge.producer_element_id,
        edge.stability,
        edge.evaluator,
        edge.backend,
        edge.tolerance,
        "stale-result-fingerprint",
        edge.corroborating_geometry_fingerprint,
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: stale_finish,
            definition_id: definition,
            name: "Stale selected edge".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: base,
                edges: vec![stale_edge],
                kind: EdgeFinishKind::Fillet,
                amount: dimension(0.75),
            },
        }]))
        .unwrap();
    let stale_graph =
        ExactBRepGraph::from_snapshot(&document.current(), definition, stale_finish).unwrap();
    assert_geometry_error(
        supervisor
            .evaluate_exact_brep_graph(&stale_graph)
            .unwrap_err(),
        "invalid_parameter",
    );
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&base_graph).unwrap(),
        base_package
    );
}

#[test]
fn worker_preserves_cubic_sketch_region_hole_volume_and_result_identity() {
    let definition = DefinitionId(92);
    let workplane = FeatureId(920);
    let sketch_id = FeatureId(921);
    let pad = FeatureId(922);
    let sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-20.0, -15.0],
                end_mm: [20.0, -15.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [20.0, -15.0],
                end_mm: [20.0, 15.0],
            },
            SketchEntity::CubicBezier {
                id: SketchEntityId(3),
                start_mm: [20.0, 15.0],
                control_1_mm: [10.0, 25.0],
                control_2_mm: [-10.0, 25.0],
                end_mm: [-20.0, 15.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [-20.0, 15.0],
                end_mm: [-20.0, -15.0],
            },
            SketchEntity::Circle {
                id: SketchEntityId(5),
                center_mm: [0.0, 0.0],
                radius_mm: 5.0,
            },
        ],
        constraints: Vec::new(),
    };
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].holes.len(), 1);
    let region = regions[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Worker compound region".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch_id,
                definition_id: definition,
                name: "Line-cubic profile with centered hole".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Compound Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: sketch_id,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(12.0)),
                }),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, pad).unwrap();
    let ExactBRepPlanarGeometry::Region { outer, holes } = &graph.profiles[0].geometry else {
        panic!("compound sketch must reach the worker as a planar region");
    };
    assert!(matches!(
        outer,
        ExactBRepPlanarLoop::Boundary { segments } if segments.len() == 4
    ));
    assert_eq!(holes.len(), 1);

    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(
        supervisor.evaluate_exact_brep_graph(&graph).unwrap(),
        package
    );
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id.0, pad.0);
    assert_eq!(package.topology_counts[4], 1);
    assert_bounds_close(package.bounds_mm, [-20.0, -15.0, 0.0, 20.0, 22.5, 12.0]);
    let expected_volume = (1_410.0 - std::f64::consts::PI * 5.0 * 5.0) * 12.0;
    assert!(package.volume_mm3 < 1_410.0 * 12.0);
    assert!(
        (package.volume_mm3 - expected_volume).abs() <= 2.0e-7,
        "worker volume {} differed from analytic compound-region volume {expected_volume}",
        package.volume_mm3
    );

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    let reopened_snapshot = reopened.snapshot();
    let reopened_graph =
        ExactBRepGraph::from_snapshot(&reopened_snapshot, definition, pad).unwrap();
    assert_eq!(reopened_graph, graph);
    assert_eq!(
        supervisor
            .evaluate_exact_brep_graph(&reopened_graph)
            .unwrap(),
        package
    );
    assert!(package.is_current(&reopened_snapshot));
}

#[test]
fn worker_cuts_a_compound_sketch_pocket_while_preserving_its_inner_island() {
    let definition = DefinitionId(93);
    let base_plane = FeatureId(929);
    let base_sketch_id = FeatureId(930);
    let base = FeatureId(931);
    let face_plane = FeatureId(932);
    let pocket_sketch_id = FeatureId(933);
    let pocket = FeatureId(934);
    let base_sketch = SketchSpec {
        workplane: base_plane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [50.0, 0.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [50.0, 0.0],
                end_mm: [50.0, 40.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [50.0, 40.0],
                end_mm: [0.0, 40.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [0.0, 40.0],
                end_mm: [0.0, 0.0],
            },
        ],
        constraints: Vec::new(),
    };
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Compound pocket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_plane,
                definition_id: definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: base_sketch_id,
                definition_id: definition,
                name: "Base sketch".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: base,
                definition_id: definition,
                name: "Base".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: base_sketch_id,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                }),
            },
        ]))
        .unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let base_request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&document.current(), definition, base)
            .unwrap();
    let base_package = supervisor.evaluate_rectangle(&base_request).unwrap();
    let top = base_package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in base_package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }

    let pocket_sketch = SketchSpec {
        workplane: face_plane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [5.0, 5.0],
                end_mm: [45.0, 5.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [45.0, 5.0],
                end_mm: [45.0, 35.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [45.0, 35.0],
                end_mm: [5.0, 35.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [5.0, 35.0],
                end_mm: [5.0, 5.0],
            },
            SketchEntity::Circle {
                id: SketchEntityId(5),
                center_mm: [25.0, 20.0],
                radius_mm: 5.0,
            },
        ],
        constraints: Vec::new(),
    };
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: face_plane,
                definition_id: definition,
                name: "Base top".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [0.0, 0.0, 10.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: pocket_sketch_id,
                definition_id: definition,
                name: "Pocket region".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pocket,
                definition_id: definition,
                name: "Compound Pocket".into(),
                kind: FeatureKind::SketchPocket(PocketSpec {
                    target: base,
                    sketch: pocket_sketch_id,
                    region: pocket_region,
                    support: Box::new(top),
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::Blind(dimension(4.0)),
                }),
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, pocket).unwrap();
    assert!(matches!(
        graph.profiles.last().map(|profile| &profile.geometry),
        Some(ExactBRepPlanarGeometry::Region { holes, .. }) if holes.len() == 1
    ));
    let result = supervisor.evaluate_exact_brep_graph(&graph).unwrap();
    let removed_volume = (40.0 * 30.0 - std::f64::consts::PI * 5.0 * 5.0) * 4.0;
    let expected_volume = 50.0 * 40.0 * 10.0 - removed_volume;
    assert!(
        (result.volume_mm3 - expected_volume).abs() <= 2.0e-7,
        "compound pocket volume {} differed from {expected_volume}",
        result.volume_mm3
    );
    assert_bounds_close(result.bounds_mm, [0.0, 0.0, 0.0, 50.0, 40.0, 10.0]);
    assert_eq!(result.topology_counts[4], 1);
    assert_eq!(result.identity.producer_feature_id.0, pocket.0);
}
