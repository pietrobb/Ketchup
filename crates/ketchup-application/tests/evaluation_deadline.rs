use ketchup_application::evaluation::{
    EvidenceStatus, ExactEvaluationSelection, ProducerKey, exact_source, exact_worker_candidates,
};
use ketchup_application::{DocumentSession, SessionError, SessionSettings};
use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadPartFeature,
    AssistantPrincipalPlane, AssistantSketchConstraint, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, FeatureId, FeatureKind,
    MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, OccurrenceId, Snapshot, Transform,
};
use ketchup_core::exact_product::ExactResultRegistry;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

fn program() -> AssistantCadEditProgram {
    program_at("Deadline test part", [0.0, 0.0, 0.0])
}

fn program_at(name: &str, translation_mm: [f64; 3]) -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: name.into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 12.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 12.0,
            }],
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
            translation_mm,
            rotation: None,
        }],
    }
}

fn worker_session() -> DocumentSession {
    let path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    DocumentSession::new(SessionSettings {
        exact_worker_path: Some(path),
        ..SessionSettings::default()
    })
}

fn add_part(session: &mut DocumentSession) {
    session
        .apply_cad_program(&program(), &BTreeSet::new())
        .unwrap();
}

fn canonical_state(session: &DocumentSession) -> (u64, String, usize, usize, bool) {
    (
        session.snapshot().revision_id(),
        session.snapshot().canonical_digest(),
        session.visible_undo_steps(),
        session.visible_redo_steps(),
        session.is_modified(),
    )
}

fn registry_fingerprints(
    snapshot: &Snapshot,
    registry: &ExactResultRegistry,
) -> BTreeMap<(u64, u64, u64), String> {
    registry
        .body_values(snapshot)
        .unwrap()
        .into_iter()
        .map(|(key, package)| {
            (
                (
                    key.definition_id.0,
                    key.body_id.0,
                    key.producer_feature_id.0,
                ),
                package.result_key().result_fingerprint.clone(),
            )
        })
        .collect()
}

fn assert_timeout_unchanged(session: &mut DocumentSession, timeout: Duration) {
    let before = canonical_state(session);
    // Include registry stamps and packages, not just counts: even replacing cached
    // evidence with an equivalent publication must not escape the deadline guard.
    let render = format!("{:?}", session.exact_results());
    let topology = format!("{:?}", session.topology_results());
    assert!(matches!(
        session.evaluate_with_timeout(timeout),
        Err(SessionError::Evaluation(reason)) if reason == "exact evaluation timed out"
    ));
    assert_eq!(canonical_state(session), before);
    assert_eq!(format!("{:?}", session.exact_results()), render);
    assert_eq!(format!("{:?}", session.topology_results()), topology);
}

#[test]
fn physical_recipe_save_open_history_recomputes_full_exact_without_cached_evidence() {
    use ketchup_core::assembly_recipe::*;
    use ketchup_core::assistant_sidecar::{
        AssistantDowelJointFace, AssistantInstancePath, AssistantStandardDowel,
    };
    use ketchup_core::document::{
        ClassificationCategoryId, ClassificationDimensionId, FeatureParameterTarget, InstancePath,
        ParameterValueType,
    };
    use ketchup_core::exact_validation::GeneralBodyParticipant;
    use ketchup_core::fabrication::{
        FABRICATION_ROLE_DIMENSION_V1, TIMBER_MEMBER_ROLE_V1, project_general_fabrication,
    };
    use ketchup_core::joinery::{DowelJointId, project_dowel_joint_contract};
    use ketchup_core::prismatic::TolerancePolicy;

    let key = |name: &str| RecipeKey::new(name).unwrap();
    let mut session = worker_session();
    let mut commands = vec![CanonicalCommand::UpsertClassificationDimension {
        id: ClassificationDimensionId(1),
        name: FABRICATION_ROLE_DIMENSION_V1.into(),
        categories: vec![(ClassificationCategoryId(1), TIMBER_MEMBER_ROLE_V1.into())],
    }];
    for (id, name, z) in [(1, "lower", 0.0), (2, "upper", 18.0)] {
        commands.extend([
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(id),
                name: name.into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(id * 2 - 1),
                definition_id: DefinitionId(id),
                name: format!("{name} profile"),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(id * 2),
                definition_id: DefinitionId(id),
                name: format!("{name} solid"),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(id * 2 - 1),
                    height: Dimension::new("18", 18.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(id),
                name: name.into(),
                transform: Transform::from_translation(0.0, 0.0, z).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(id),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetProductionCode {
                instance_path: InstancePath::root(OccurrenceId(id)),
                code: Some(format!("{id:012}")),
            },
        ]);
    }
    let proposal = session.plan_commands(CommandBatch::new(commands)).unwrap();
    session.apply_proposal(&proposal).unwrap();
    let face = |id, z, normal| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: id,
            steps: vec![],
        },
        face_origin_local_mm: [0.0, 0.0, z],
        inward_unit_local: normal,
        bounds_min_local_mm: [0.0; 3],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::CreatePhysicalDowelJoint {
                    joint_id: None,
                    name: "row".into(),
                    first: face(1, 18.0, [0.0, 0.0, -1.0]),
                    second: face(2, 0.0, [0.0, 0.0, 1.0]),
                    first_center_local_mm: [20.0, 20.0, 18.0],
                    row_unit_first_local: [1.0, 0.0, 0.0],
                    count: 2,
                    spacing_mm: 32.0,
                    dowel: AssistantStandardDowel::D8x30,
                }],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let snapshot = session.snapshot();
    let parts = [("lower", OccurrenceId(1)), ("upper", OccurrenceId(2))]
        .into_iter()
        .map(|(name, id)| {
            let definition = snapshot.occurrence(id).unwrap().definition_id();
            let mut parameters = BTreeMap::new();
            let features = snapshot
                .features()
                .filter(|f| f.definition_id() == definition)
                .map(|feature| {
                    let kind = match feature.kind() {
                        FeatureKind::Profile { .. } => RecognizedRecipeFeatureKind::Profile,
                        FeatureKind::Extrusion { .. } => {
                            parameters.insert(
                                key("height"),
                                RecipeParameter {
                                    value: 18.0,
                                    unit: RecipeParameterUnit::Millimetres,
                                    target: Some(
                                        FeatureParameterTarget::new(
                                            feature.id(),
                                            "height",
                                            ParameterValueType::Length,
                                        )
                                        .unwrap(),
                                    ),
                                },
                            );
                            RecognizedRecipeFeatureKind::Extrusion
                        }
                        FeatureKind::Workplane(_) => RecognizedRecipeFeatureKind::Workplane,
                        FeatureKind::Sketch(_) => RecognizedRecipeFeatureKind::Sketch,
                        FeatureKind::Pocket { .. } => RecognizedRecipeFeatureKind::Pocket,
                        other => panic!("unexpected panel feature: {other:?}"),
                    };
                    (
                        key(&format!("{name}/{}", feature.id().0)),
                        feature.id(),
                        kind,
                    )
                })
                .collect();
            RecipePartAdoption {
                key: key(name),
                instance_path: InstancePath::root(id),
                mobility: RecipePartMobility::Fixed,
                edit_scope: RecipeEditScope::Occurrence(InstancePath::root(id)),
                parameters,
                features,
            }
        })
        .collect();
    let recipe = AssemblyRecipe::adopt(
        &snapshot,
        key("persistence"),
        parts,
        vec![RecipeRelation {
            key: key("contact"),
            kind: RecipeRelationKind::Contact,
            first: RecipeFaceRef {
                part: key("lower"),
                role: "bounds.z.maximum".into(),
            },
            second: RecipeFaceRef {
                part: key("upper"),
                role: "bounds.z.minimum".into(),
            },
        }],
        vec![RecipeJoinery {
            key: key("row"),
            first_part: key("lower"),
            second_part: key("upper"),
            dowel_joint_id: DowelJointId(1),
        }],
    )
    .unwrap();
    let adoption = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    session.apply_proposal(&adoption).unwrap();
    assert!(session.evaluate().unwrap().establishes_full_baseline());
    let before = session.snapshot();
    let before_exact = registry_fingerprints(&before, session.exact_results());
    let before_topology = registry_fingerprints(&before, session.topology_results());
    let undo_before = session.visible_undo_steps();
    let patch = RecipeSemanticPatch {
        nodes: vec![RecipePatchNode {
            key: key("grow"),
            dependencies: vec![],
            change: RecipeSemanticChange::SetParameter {
                part: key("lower"),
                parameter: key("height"),
                value: 28.0,
                anchor: RecipeDimensionAnchor::Maximum,
            },
        }],
    };
    let compiled = compile_assembly_recipe_patch(&before, &patch).unwrap();
    let proposal = session.plan_commands(compiled.batch.unwrap()).unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert_eq!(session.visible_undo_steps(), undo_before + 1);
    assert!(session.evaluate().unwrap().establishes_full_baseline());
    let after = session.snapshot();
    let after_exact = registry_fingerprints(&after, session.exact_results());
    let after_topology = registry_fingerprints(&after, session.topology_results());
    assert_ne!(before_exact, after_exact);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("physical-recipe.ketchup");
    session
        .save(&path, ketchup_application::SaveOptions { overwrite: false })
        .unwrap();
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert!(reopened.exact_results().is_empty());
    assert!(reopened.topology_results().is_empty());
    assert_eq!(reopened.visible_undo_steps(), undo_before + 1);
    for (stage, expected, render, topology) in [
        ("open", &after, &after_exact, &after_topology),
        ("undo", &before, &before_exact, &before_topology),
        ("redo", &after, &after_exact, &after_topology),
    ] {
        match stage {
            "undo" => {
                reopened.undo().unwrap();
                assert_eq!(reopened.visible_redo_steps(), 1);
            }
            "redo" => {
                reopened.redo().unwrap();
                assert_eq!(reopened.visible_undo_steps(), undo_before + 1);
            }
            _ => {}
        }
        let actual = reopened.snapshot();
        assert_eq!(
            actual.canonical_digest(),
            expected.canonical_digest(),
            "{stage}"
        );
        assert_eq!(
            actual.assembly_recipe(),
            expected.assembly_recipe(),
            "{stage}"
        );
        assert_eq!(
            actual.dowel_joint(DowelJointId(1)),
            expected.dowel_joint(DowelJointId(1))
        );
        let projection =
            project_dowel_joint_contract(&actual, actual.dowel_joint(DowelJointId(1)).unwrap())
                .unwrap();
        assert_eq!(projection.pairs.len(), 2);
        for pair in projection.pairs {
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
            assert_eq!(pair.first.shared_center_world_mm[2], 18.0);
            assert_eq!(
                pair.physical_probe_coincidence
                    .unwrap()
                    .maximum_endpoint_error_mm,
                0.0
            );
        }
        // A new session has no render/topology evidence from the edited session or history.
        let cold_path = directory.path().join(format!("{stage}.ketchup"));
        reopened
            .save(
                &cold_path,
                ketchup_application::SaveOptions { overwrite: false },
            )
            .unwrap();
        let mut cold = DocumentSession::open(&cold_path, SessionSettings::default()).unwrap();
        let task = cold.start_exact_evaluation_task();
        assert_eq!(task.selection, ExactEvaluationSelection::Full);
        assert_eq!(task.progress().reused_producers, 0);
        assert!(task.progress().total_producers >= 2);
        let products = task.wait(Duration::from_secs(30)).unwrap();
        assert!(
            cold.publish_exact_evaluation(&task, products)
                .unwrap()
                .establishes_full_baseline()
        );
        assert_eq!(
            &registry_fingerprints(&cold.snapshot(), cold.exact_results()),
            render,
            "{stage}"
        );
        assert_eq!(
            &registry_fingerprints(&cold.snapshot(), cold.topology_results()),
            topology,
            "{stage}"
        );
        let snapshot = cold.snapshot();
        let tolerance = TolerancePolicy::default();
        let participants = [1, 2].map(|id| {
            GeneralBodyParticipant::accept(
                &snapshot,
                cold.exact_results(),
                InstancePath::root(OccurrenceId(id)),
                tolerance,
            )
            .unwrap()
        });
        let validation =
            ketchup_application::validation::fabrication_collision_validation_with_worker(
                &snapshot,
                &participants,
                cold.container_data(),
                None,
                Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(
            validation.report.state,
            ketchup_core::validation::ValidationState::Passed
        );
        let fabrication = project_general_fabrication(
            &snapshot,
            cold.exact_results(),
            &validation.cases,
            &validation.report,
            tolerance,
        )
        .unwrap();
        assert!(fabrication.bom.envelope.is_current(&snapshot));
        assert_eq!(fabrication.bom.rows.len(), 2);
        assert_eq!(
            fabrication
                .bom
                .rows
                .iter()
                .map(|row| row.quantity)
                .sum::<usize>(),
            2
        );
        let expected_height = if stage == "undo" { 18.0 } else { 28.0 };
        for row in &fabrication.bom.rows {
            assert_eq!(row.dimensions.length_mm, 100.0);
            assert_eq!(row.dimensions.width_mm, 50.0);
            assert_eq!(
                row.dimensions.height_mm,
                if row.definition_id == DefinitionId(1) {
                    expected_height
                } else {
                    18.0
                }
            );
        }
        let bom = fabrication.bom_export(&snapshot).unwrap();
        let machining = fabrication.manufacturing_export(&snapshot).unwrap();
        assert!(!bom.is_empty() && !machining.is_empty());
        let job = fabrication.production_job(&snapshot, &[]).unwrap();
        assert_eq!(job["source_digest"], snapshot.canonical_digest());
        assert_eq!(job["source_revision"], snapshot.revision_id());
        let parts = job["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        for part in parts {
            let lower = part["definition_id"] == 1;
            assert_eq!(
                part["dimensions_mm"],
                serde_json::json!([100.0, 50.0, if lower { expected_height } else { 18.0 }])
            );
            assert_eq!(part["code"].as_str().unwrap().len(), 12);
            let holes = part["dowel_holes"].as_array().unwrap();
            assert_eq!(holes.len(), 2);
            for (hole, x) in holes.iter().zip([20.0, 52.0]) {
                assert_eq!(hole["diameter_mm"], 8.0);
                assert_eq!(hole["depth_mm"], 16.0);
                assert_eq!(
                    hole["entry_local_mm"],
                    serde_json::json!([x, 20.0, if lower { expected_height } else { 0.0 }])
                );
                assert_eq!(
                    hole["inward_unit_local"],
                    serde_json::json!([0.0, 0.0, if lower { -1.0 } else { 1.0 }])
                );
            }
        }
        // Previously generated exports cannot be reused against the opposite history state.
        let other = if stage == "undo" { &after } else { &before };
        assert!(fabrication.bom_export(other).is_err());
        assert!(fabrication.manufacturing_export(other).is_err());
        assert!(fabrication.production_job(other, &[]).is_err());
    }
}

#[test]
fn default_evaluation_works_and_zero_refuses_even_current_results() {
    assert_eq!(
        SessionSettings::default().evaluation_timeout,
        Duration::from_secs(30)
    );
    let mut session = worker_session();
    assert_timeout_unchanged(&mut session, Duration::ZERO);
    add_part(&mut session);
    assert_timeout_unchanged(&mut session, Duration::ZERO);
    assert!(session.exact_results().is_empty());
    assert!(session.topology_results().is_empty());

    let before = canonical_state(&session);
    let report = session.evaluate().unwrap();
    assert!(report.complete, "{report:?}");
    assert!(report.topology_complete, "{report:?}");
    assert_eq!(report.source, exact_source(&session.snapshot()));
    assert_eq!(canonical_state(&session), before);
    assert!(!session.exact_results().is_empty());
    assert!(!session.topology_results().is_empty());
    assert_timeout_unchanged(&mut session, Duration::ZERO);

    let cached = session.evaluate().unwrap();
    assert!(cached.complete && cached.topology_complete, "{cached:?}");
    assert!(cached.producers.iter().all(|entry| {
        entry.render == EvidenceStatus::Current && entry.topology == EvidenceStatus::Current
    }));
    assert_eq!(canonical_state(&session), before);
}

#[test]
fn metadata_only_edit_reuses_exact_products_without_duplicate_materialization() {
    let mut session = worker_session();
    add_part(&mut session);
    let initial = session.evaluate().unwrap();
    assert!(initial.complete && initial.topology_complete, "{initial:?}");
    let occurrence_id = session.snapshot().occurrences().next().unwrap().id();
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor {
                id: occurrence_id,
                color: Some([73, 109, 151]),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();

    let task = session.start_exact_evaluation_task();
    assert_eq!(task.progress().total_producers, 1);
    assert_eq!(task.progress().reused_producers, 1);
    let products = task.wait(Duration::from_secs(30)).unwrap();
    let report = session.publish_exact_evaluation(&task, products).unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
}

#[test]
fn session_reuses_only_a_complete_full_baseline_for_incremental_exact() {
    let mut session = worker_session();
    add_part(&mut session);
    session
        .apply_cad_program(
            &program_at("Independent part", [100.0, 0.0, 0.0]),
            &BTreeSet::new(),
        )
        .unwrap();
    let full = session.evaluate().unwrap();
    assert!(full.establishes_full_baseline(), "{full:?}");
    assert_eq!(full.producers.len(), 2, "{full:?}");
    let extrusion = session
        .snapshot()
        .features()
        .find(|feature| feature.kind().produces_body())
        .unwrap()
        .id();

    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: extrusion,
                dimension: Dimension::new("31", 31.0).unwrap(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    let plan = session.incremental_exact_plan().unwrap();
    assert!(plan.baseline_reused, "{plan:?}");
    let ExactEvaluationSelection::Scoped(scope) = &plan.selection else {
        panic!("complete baseline must enable a scoped plan: {plan:?}");
    };
    let scope = scope.clone();
    assert_eq!(scope.len(), 1, "{plan:?}");

    let task = session.start_incremental_exact_evaluation_task();
    assert_eq!(task.selection, ExactEvaluationSelection::Scoped(scope));
    assert_eq!(task.progress().total_producers, 1);
    let products = task.wait(Duration::from_secs(30)).unwrap();
    let scoped = session.publish_exact_evaluation(&task, products).unwrap();
    assert!(scoped.complete && scoped.topology_complete, "{scoped:?}");
    assert!(matches!(
        scoped.selection,
        ExactEvaluationSelection::Scoped(_)
    ));
    let scoped_render = registry_fingerprints(&session.snapshot(), session.exact_results());
    let scoped_topology = registry_fingerprints(&session.snapshot(), session.topology_results());
    assert_eq!(scoped_render.len(), 2);
    assert_eq!(scoped_topology.len(), 2);

    let full_task = session.start_exact_evaluation_task();
    assert_eq!(full_task.selection, ExactEvaluationSelection::Full);
    assert_eq!(full_task.progress().total_producers, 2);
    assert_eq!(full_task.progress().reused_producers, 2);
    let full_products = full_task.wait(Duration::from_secs(30)).unwrap();
    let full_after_scoped = session
        .publish_exact_evaluation(&full_task, full_products)
        .unwrap();
    assert!(full_after_scoped.establishes_full_baseline());
    assert_eq!(full_after_scoped.producers.len(), 2);
    assert_eq!(
        registry_fingerprints(&session.snapshot(), session.exact_results()),
        scoped_render
    );
    assert_eq!(
        registry_fingerprints(&session.snapshot(), session.topology_results()),
        scoped_topology
    );

    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: extrusion,
                dimension: Dimension::new("32", 32.0).unwrap(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert!(session.incremental_exact_plan().unwrap().baseline_reused);
    let second_scoped_task = session.start_incremental_exact_evaluation_task();
    assert_eq!(second_scoped_task.progress().total_producers, 1);
    let second_scoped_products = second_scoped_task.wait(Duration::from_secs(30)).unwrap();
    session
        .publish_exact_evaluation(&second_scoped_task, second_scoped_products)
        .unwrap();

    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: extrusion,
                dimension: Dimension::new("33", 33.0).unwrap(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    let fallback = session.incremental_exact_plan().unwrap();
    assert!(!fallback.baseline_reused, "{fallback:?}");
    assert_eq!(fallback.selection, ExactEvaluationSelection::Full);
    assert_eq!(
        fallback.fallback_reason.as_deref(),
        Some("missing or stale complete exact baseline")
    );
}

#[test]
fn incremental_unsupported_producer_is_incomplete_and_cannot_establish_baseline() {
    let mut session = worker_session();
    add_part(&mut session);
    assert!(session.evaluate().unwrap().establishes_full_baseline());

    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(100),
                name: "Unsupported mesh".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(100),
                definition_id: DefinitionId(100),
                name: "Mesh".into(),
                kind: FeatureKind::MeshBody(MeshBodySpec {
                    schema: MESH_BODY_SCHEMA_V1.into(),
                    vertices_mm: vec![
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                    ],
                    triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
                    authority: MeshAuthority::Authored {
                        provenance: "incremental-unsupported-test".into(),
                    },
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(100),
                definition_id: DefinitionId(100),
                name: "Unsupported mesh".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    let plan = session.incremental_exact_plan().unwrap();
    assert!(plan.baseline_reused, "{plan:?}");
    assert!(matches!(
        plan.selection,
        ExactEvaluationSelection::Scoped(_)
    ));

    let task = session.start_incremental_exact_evaluation_task();
    let products = task.wait(Duration::from_secs(30)).unwrap();
    let report = session.publish_exact_evaluation(&task, products).unwrap();
    assert!(!report.complete, "{report:?}");
    assert!(!report.topology_complete, "{report:?}");
    assert_eq!(report.producers.len(), 1, "{report:?}");
    assert!(matches!(
        &report.producers[0].render,
        EvidenceStatus::NotEvaluated { reason } if reason == "unsupported producer"
    ));
    assert!(matches!(
        &report.producers[0].topology,
        EvidenceStatus::NotEvaluated { reason } if reason == "unsupported producer"
    ));
    assert!(!report.establishes_full_baseline());
}

#[cfg(windows)]
#[test]
fn crashed_worker_cannot_pass_a_scoped_exact_evaluation() {
    let directory = tempfile::tempdir().unwrap();
    let worker = directory.path().join("crash-worker.cmd");
    std::fs::write(&worker, "@exit /b 23\r\n").unwrap();
    let mut session = DocumentSession::new(SessionSettings {
        exact_worker_path: Some(worker),
        ..SessionSettings::default()
    });
    add_part(&mut session);
    let snapshot = session.snapshot();
    let producer = snapshot
        .features()
        .find(|feature| feature.kind().produces_body())
        .unwrap();
    let scope = BTreeSet::from([ProducerKey {
        definition_id: producer.definition_id(),
        feature_id: producer.id(),
    }]);

    let task = session.start_scoped_exact_evaluation_task(Some(&scope));
    let products = task.wait(Duration::from_secs(30)).unwrap();
    let report = session.publish_exact_evaluation(&task, products).unwrap();
    assert_eq!(report.selection, ExactEvaluationSelection::Scoped(scope));
    assert!(!report.complete, "{report:?}");
    assert!(!report.topology_complete, "{report:?}");
    assert!(
        report
            .producers
            .iter()
            .all(|producer| !producer.render.is_evaluated() || !producer.topology.is_evaluated()),
        "{report:?}"
    );
    assert!(!report.establishes_full_baseline());
}

#[test]
fn cancelled_incremental_task_cannot_publish_or_establish_baseline() {
    let mut session = worker_session();
    add_part(&mut session);
    assert!(session.evaluate().unwrap().establishes_full_baseline());
    let extrusion = session
        .snapshot()
        .features()
        .find(|feature| feature.kind().produces_body())
        .unwrap()
        .id();
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: extrusion,
                dimension: Dimension::new("31", 31.0).unwrap(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    let before_render = format!("{:?}", session.exact_results());
    let before_topology = format!("{:?}", session.topology_results());
    assert!(matches!(
        session.evaluate_incremental_with_timeout(Duration::from_nanos(1)),
        Err(SessionError::Evaluation(reason)) if reason == "exact evaluation timed out"
    ));
    assert_eq!(format!("{:?}", session.exact_results()), before_render);
    assert_eq!(format!("{:?}", session.topology_results()), before_topology);
    assert!(session.incremental_exact_plan().is_some());

    let task = session.start_incremental_exact_evaluation_task();
    task.cancel();
    assert!(matches!(
        task.wait(Duration::from_secs(30)),
        Err(reason) if reason == "exact evaluation cancelled"
    ));
    assert_eq!(format!("{:?}", session.exact_results()), before_render);
    assert_eq!(format!("{:?}", session.topology_results()), before_topology);
    assert!(session.incremental_exact_plan().is_some());
}

#[test]
fn short_deadline_cannot_publish_late_or_poison_real_worker_retry() {
    let mut session = worker_session();
    add_part(&mut session);
    let populated = canonical_state(&session);
    // Preparation alone exceeds this positive budget. Queued results must not
    // turn recv_timeout(0) into success after the overall budget is exhausted.
    assert_timeout_unchanged(&mut session, Duration::from_nanos(1));
    assert!(session.exact_results().is_empty());
    assert!(session.topology_results().is_empty());

    session.undo().unwrap();
    let empty = canonical_state(&session);
    assert_eq!(session.visible_redo_steps(), 1);
    assert_timeout_unchanged(&mut session, Duration::ZERO);
    let report = session.evaluate().unwrap();
    assert_eq!(report.source, exact_source(&session.snapshot()));
    assert!(!report.complete);
    assert!(report.producers.is_empty());
    assert_eq!(canonical_state(&session), empty);
    assert!(session.exact_results().is_empty());
    assert!(session.topology_results().is_empty());

    session.redo().unwrap();
    assert_eq!(canonical_state(&session), populated);
    let report = session.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
    assert_eq!(report.source, exact_source(&session.snapshot()));
    assert_eq!(canonical_state(&session), populated);
    for registry in [session.exact_results(), session.topology_results()] {
        assert!(!registry.is_empty());
        assert!(registry.is_bound_to(&session.snapshot()));
        assert!(
            registry
                .values()
                .all(|package| package.is_current(&session.snapshot()))
        );
    }
    assert_timeout_unchanged(&mut session, Duration::from_nanos(1));
    assert!(session.evaluate().unwrap().complete);
    assert_eq!(canonical_state(&session), populated);
}

#[test]
fn per_call_override_does_not_change_the_settings_default() {
    let mut session = DocumentSession::new(SessionSettings {
        evaluation_timeout: Duration::ZERO,
        ..SessionSettings::default()
    });
    let before = canonical_state(&session);
    assert!(matches!(
        session.evaluate(),
        Err(SessionError::Evaluation(_))
    ));
    let report = session
        .evaluate_with_timeout(Duration::from_secs(30))
        .unwrap();
    assert!(!report.complete);
    assert!(report.producers.is_empty());
    assert!(matches!(
        session.evaluate(),
        Err(SessionError::Evaluation(_))
    ));
    assert_eq!(canonical_state(&session), before);
}

#[test]
fn redo_count_tracks_authoritative_history_including_branching() {
    let mut session = DocumentSession::default();
    assert_eq!(session.visible_redo_steps(), 0);
    add_part(&mut session);
    session.set_grounded(OccurrenceId(1), true).unwrap();
    assert_eq!(session.visible_undo_steps(), 2);
    assert_eq!(session.visible_redo_steps(), 0);
    session.undo().unwrap();
    assert_eq!(session.visible_redo_steps(), 1);
    session.undo().unwrap();
    assert_eq!(session.visible_redo_steps(), 2);
    session.redo().unwrap();
    assert_eq!(session.visible_redo_steps(), 1);
    session.set_grounded(OccurrenceId(1), true).unwrap();
    assert_eq!(session.visible_redo_steps(), 0);
    assert!(matches!(session.redo(), Err(SessionError::NoRedo)));
    assert_eq!(session.visible_undo_steps(), 2);
}
