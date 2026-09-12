use ketchup_application::evaluation::*;
use ketchup_application::{
    AssistantValidationSelection, DocumentSession, SaveOptions, SessionError, SessionSettings,
    StructuralValidationScope,
    model_query::{EntityKind, ModelQuery, PageRequest},
    scoped_static_load_report,
};
use ketchup_core::{
    assistant_sidecar::*,
    document::*,
    exact_product::{ExactBodyPackage, ExactResultRegistry},
    persistence::ContainerData,
};
use std::{collections::BTreeSet, time::Duration};
#[test]
fn evaluation_retry_distinguishes_failed_topology_from_unsupported_topology() {
    let mut report = ketchup_application::evaluation::EvaluationReport {
        source: exact_source(&DocumentStore::new().current()),
        producers: vec![ProducerCoverage {
            key: ProducerKey {
                definition_id: DefinitionId(1),
                feature_id: FeatureId(2),
            },
            render: EvidenceStatus::Current,
            topology: EvidenceStatus::NotEvaluated {
                reason: "topology not provided by this request".into(),
            },
        }],
        complete: true,
        topology_complete: false,
        not_evaluated: None,
    };
    assert!(
        !report.needs_retry(),
        "unsupported topology must not retry forever"
    );
    report.producers[0].topology = EvidenceStatus::Failed {
        reason: "worker disconnected".into(),
    };
    assert!(report.needs_retry());
    report.producers[0].topology = EvidenceStatus::Evaluated;
    report.topology_complete = true;
    assert!(!report.needs_retry());
    report.complete = false;
    report.producers[0].render = EvidenceStatus::Failed {
        reason: "render failed".into(),
    };
    assert!(report.needs_retry());
}

fn part() -> AssistantCadEditOperation {
    AssistantCadEditOperation::CreatePart {
        name: "Editable part".into(),
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
        translation_mm: [5.0, 6.0, 7.0],
        rotation: None,
    }
}

fn program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![part()],
    }
}
fn worker_settings() -> SessionSettings {
    let path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    SessionSettings {
        exact_worker_path: Some(path),
        evaluation_timeout: Duration::from_secs(30),
    }
}

#[test]
fn shared_python_contract_corpus_plans_and_evaluates_with_real_worker() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/assistant_cad_contract_corpus.json"
    ))
    .unwrap();
    assert_eq!(corpus["schema"], "ketchup.assistant-cad-contract-corpus.v1");
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2);
    for case in corpus["validation_cases"].as_array().unwrap() {
        let program: AssistantCadEditProgram =
            serde_json::from_value(case["program"].clone()).unwrap();
        assert_eq!(
            program.validate().is_ok(),
            case["valid"].as_bool().unwrap(),
            "{case}"
        );
    }

    for case in cases {
        let mut session = DocumentSession::new(worker_settings());
        for field in ["setup_program", "program"] {
            let Some(value) = case.get(field) else {
                continue;
            };
            let program: AssistantCadEditProgram = serde_json::from_value(value.clone()).unwrap();
            program.validate().unwrap();
            session
                .apply_cad_program(&program, &BTreeSet::new())
                .unwrap();
        }
        let report = session.evaluate().unwrap();
        assert!(
            report.complete && report.topology_complete,
            "{case}: {report:?}"
        );
        if let Some(expected_color) = case.get("expected_color") {
            let expected_color: [u8; 3] = serde_json::from_value(expected_color.clone()).unwrap();
            assert_eq!(
                session
                    .snapshot()
                    .occurrence(OccurrenceId(1))
                    .unwrap()
                    .color(),
                Some(expected_color)
            );
        }
    }
}

#[test]
fn explicit_current_snapshot_save_is_atomic_and_resets_the_persistent_history_boundary() {
    let mut session = DocumentSession::default();
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "parameter".into(),
                dimension: Dimension::new("10", 10.0).unwrap(),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    let expected = session.snapshot();
    assert_eq!(session.visible_undo_steps(), 2);

    let directory = tempfile::tempdir().unwrap();
    let occupied = directory.path().join("occupied.ketchup");
    std::fs::write(&occupied, b"existing").unwrap();
    assert!(
        session
            .save_current_snapshot(&occupied, SaveOptions::default())
            .is_err()
    );
    assert_eq!(std::fs::read(&occupied).unwrap(), b"existing");
    assert_eq!(session.visible_undo_steps(), 2);
    assert!(session.is_modified());

    let path = directory.path().join("current.ketchup");
    session
        .save_current_snapshot(&path, SaveOptions::default())
        .unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        expected.canonical_digest()
    );
    assert_eq!(session.snapshot().revision_id(), expected.revision_id());
    assert_eq!(session.visible_undo_steps(), 0);
    assert_eq!(session.visible_redo_steps(), 0);
    assert!(!session.is_modified());

    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        expected.canonical_digest()
    );
    assert_eq!(reopened.snapshot().revision_id(), expected.revision_id());
    assert_eq!(reopened.visible_undo_steps(), 0);
    assert_eq!(reopened.visible_redo_steps(), 0);
    let proposal = reopened
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("30", 30.0).unwrap(),
            },
        ]))
        .unwrap();
    let next = reopened.apply_proposal(&proposal).unwrap();
    assert!(next.revision_id() > expected.revision_id());
}

#[test]
fn recovered_session_requires_explicit_save_as_and_exposes_the_actual_source() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("damaged.ketchup");
    let recovery = requested.with_extension("ketchup.recovery");
    let destination = directory.path().join("recovered-copy.ketchup");
    let occupied = directory.path().join("occupied.ketchup");

    let mut author = DocumentSession::default();
    author
        .apply_cad_program(&program(), &BTreeSet::new())
        .unwrap();
    let expected = author.snapshot().canonical_digest();
    author.save(&requested, SaveOptions::default()).unwrap();
    std::fs::copy(&requested, &recovery).unwrap();
    std::fs::write(&requested, b"corrupt primary").unwrap();

    let mut recovered = DocumentSession::open(&requested, SessionSettings::default()).unwrap();
    assert_eq!(recovered.snapshot().canonical_digest(), expected);
    assert_eq!(recovered.path(), None);
    assert!(recovered.is_modified());
    let state = recovered.recovery_state().unwrap();
    assert_eq!(state.requested_path(), requested);
    assert_eq!(state.source_path(), recovery);

    std::fs::write(&occupied, b"preserve me").unwrap();
    assert!(recovered.save(&occupied, SaveOptions::default()).is_err());
    assert_eq!(std::fs::read(&occupied).unwrap(), b"preserve me");
    assert!(recovered.recovery_state().is_some());
    assert!(recovered.is_modified());

    recovered
        .save(&destination, SaveOptions::default())
        .unwrap();
    assert_eq!(recovered.path(), Some(destination.as_path()));
    assert!(recovered.recovery_state().is_none());
    assert!(!recovered.is_modified());
    assert_eq!(std::fs::read(&requested).unwrap(), b"corrupt primary");
    assert_eq!(
        DocumentSession::open(&destination, SessionSettings::default())
            .unwrap()
            .snapshot()
            .canonical_digest(),
        expected
    );
}

#[test]
fn real_worker_session_save_open_and_read_only_reports() {
    let settings = worker_settings();
    let mut session = DocumentSession::new(settings.clone());
    let empty = session.snapshot();
    let proposal = session
        .plan_cad_program(&program(), &BTreeSet::new())
        .unwrap();
    assert_eq!(session.visible_undo_steps(), 0);
    assert_eq!(
        session.snapshot().canonical_digest(),
        empty.canonical_digest()
    );
    let snapshot = session.apply_proposal(&proposal).unwrap();
    let report = session.evaluate().unwrap();
    assert!(report.complete, "{report:?}");
    assert!(report.topology_complete, "{report:?}");
    assert_eq!(report.producers.len(), 1);
    let cylindrical_face = session
        .topology_results()
        .values()
        .find_map(|package| match package.as_ref() {
            ExactBodyPackage::Graph(graph) => graph
                .face_evidence
                .iter()
                .find(|face| face.surface_kind == "cylinder"),
            _ => None,
        })
        .expect("circle extrusion publishes its cylindrical face");
    assert_eq!(cylindrical_face.unit_normal, [0.0; 3]);
    assert_eq!(cylindrical_face.unit_axis_direction, Some([0.0, 0.0, 1.0]));
    assert_eq!(session.visible_undo_steps(), 1);
    assert_eq!(
        session.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    let validation = session.validators(&AssistantValidationSelection::only(&["collision"]));
    assert_eq!(validation["canonical_digest"], snapshot.canonical_digest());
    assert_eq!(session.visible_undo_steps(), 1);
    assert!(
        session
            .evaluate()
            .unwrap()
            .producers
            .iter()
            .all(|entry| entry.render == EvidenceStatus::Current)
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(session.save(&path, SaveOptions::default()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut reopened = DocumentSession::open(&path, settings).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(reopened.visible_undo_steps(), 1);
    assert!(reopened.evaluate().unwrap().complete);
    assert_eq!(
        reopened.validators(&AssistantValidationSelection::only(&["collision"])),
        validation
    );
    reopened.undo().unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        empty.canonical_digest()
    );
    reopened.redo().unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        empty.canonical_digest()
    );
    session.redo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    session.set_grounded(OccurrenceId(1), true).unwrap();
    assert!(session.snapshot().occurrence_is_grounded(OccurrenceId(1)));
    session.undo().unwrap();
    assert!(!session.snapshot().occurrence_is_grounded(OccurrenceId(1)));
}

#[test]
fn real_worker_line_edge_resolves_the_same_shared_axis_as_direct_geometry() {
    let base_program = || AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Axis source".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![
                AssistantSketchEntity::Line {
                    id: 1,
                    start_mm: [0.0, 0.0],
                    end_mm: [12.0, 0.0],
                },
                AssistantSketchEntity::Line {
                    id: 2,
                    start_mm: [12.0, 0.0],
                    end_mm: [12.0, 8.0],
                },
                AssistantSketchEntity::Line {
                    id: 3,
                    start_mm: [12.0, 8.0],
                    end_mm: [0.0, 8.0],
                },
                AssistantSketchEntity::Line {
                    id: 4,
                    start_mm: [0.0, 8.0],
                    end_mm: [0.0, 0.0],
                },
            ],
            constraints: Vec::new(),
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 6.0 },
            translation_mm: [0.0; 3],
            rotation: None,
        }],
    };
    let helix_program = |axis| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateHelixPath {
            name: "Edge-axis helix".into(),
            parameters: AssistantHelixParameters {
                axis,
                radius_mm: 2.0,
                pitch_mm: 3.0,
                turns: 1.5,
                start_angle_degrees: 20.0,
                handedness: AssistantHelixHandedness::Right,
            },
        }],
    };

    let settings = worker_settings();
    let mut edge_session = DocumentSession::new(settings.clone());
    edge_session
        .apply_cad_program(&base_program(), &BTreeSet::new())
        .unwrap();
    let evaluated = edge_session.snapshot();
    assert!(edge_session.evaluate().unwrap().topology_complete);
    let query = ModelQuery::default();
    let page = query
        .page_with_topology(
            &evaluated,
            edge_session.topology_results(),
            &PageRequest {
                kind: EntityKind::Edges,
                limit: 100,
                search: "line".into(),
                definition_id: Some(1),
                tag_id: None,
                classification_dimension_id: None,
                classification_category_id: None,
                world_bounds_mm: None,
                cursor: None,
            },
        )
        .unwrap();
    let edge = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| {
            edge["geometry"]["axis_origin_mm"].is_array()
                && edge["geometry"]["unit_axis_direction"].is_array()
        })
        .expect("a straight exact edge must publish its own axis");
    let reference_id = edge["reference_id"].as_str().unwrap().to_owned();
    let vector = |field: &serde_json::Value| {
        [
            field[0].as_f64().unwrap(),
            field[1].as_f64().unwrap(),
            field[2].as_f64().unwrap(),
        ]
    };
    let origin_mm = vector(&edge["geometry"]["axis_origin_mm"]);
    let direction = vector(&edge["geometry"]["unit_axis_direction"]);

    edge_session
        .apply_cad_program(
            &helix_program(AssistantAxisSpec::Edge {
                edge_reference_id: reference_id.clone(),
                instance_path: None,
            }),
            &BTreeSet::new(),
        )
        .unwrap();
    let mut direct_session = DocumentSession::new(settings);
    direct_session
        .apply_cad_program(&base_program(), &BTreeSet::new())
        .unwrap();
    direct_session
        .apply_cad_program(
            &helix_program(AssistantAxisSpec::OriginDirection {
                origin_mm,
                direction,
            }),
            &BTreeSet::new(),
        )
        .unwrap();
    assert_eq!(
        edge_session
            .snapshot()
            .feature(FeatureId(4))
            .unwrap()
            .kind(),
        direct_session
            .snapshot()
            .feature(FeatureId(4))
            .unwrap()
            .kind()
    );
    assert!(
        edge_session
            .plan_cad_program(
                &helix_program(AssistantAxisSpec::Edge {
                    edge_reference_id: reference_id,
                    instance_path: None,
                }),
                &BTreeSet::new(),
            )
            .is_ok()
    );
}

#[test]
fn circular_pattern_binds_exact_edge_axis_to_the_selected_world_instance() {
    let mut session = DocumentSession::new(worker_settings());
    session
        .apply_cad_program(&program(), &BTreeSet::new())
        .unwrap();
    let setup = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Rotated axis carrier".into(),
                transform: Transform::from_matrix([
                    0.0, 0.0, 1.0, 100.0, 0.0, 1.0, 0.0, 20.0, -1.0, 0.0, 0.0, 30.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Repeated transformed axis source".into(),
                transform: Transform::from_translation(4.0, 5.0, 6.0).unwrap(),
                parent: Some(GroupId(1)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(1),
                name: "Nonuniform axis source".into(),
                transform: Transform::from_matrix([
                    2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    session.apply_proposal(&setup).unwrap();
    let evaluated = session.snapshot();
    assert!(session.evaluate().unwrap().topology_complete);

    let page = ModelQuery::default()
        .page_with_topology(
            &evaluated,
            session.topology_results(),
            &PageRequest {
                kind: EntityKind::Edges,
                limit: 100,
                search: "circle".into(),
                definition_id: Some(1),
                tag_id: None,
                classification_dimension_id: None,
                classification_category_id: None,
                world_bounds_mm: None,
                cursor: None,
            },
        )
        .unwrap();
    let edge = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| {
            edge["geometry"]["axis_origin_mm"].is_array()
                && edge["geometry"]["unit_axis_direction"].is_array()
        })
        .expect("a circular exact edge must publish its axis");
    let reference_id = edge["reference_id"].as_str().unwrap().to_owned();
    let vector = |field: &serde_json::Value| {
        [
            field[0].as_f64().unwrap(),
            field[1].as_f64().unwrap(),
            field[2].as_f64().unwrap(),
        ]
    };
    let local_origin = vector(&edge["geometry"]["axis_origin_mm"]);
    let local_direction = vector(&edge["geometry"]["unit_axis_direction"]);
    let world = evaluated
        .resolve_instance_path(&InstancePath::root(OccurrenceId(2)))
        .unwrap()
        .world_transform;
    let matrix = world.matrix();
    let world_origin = [0, 1, 2].map(|row| {
        matrix[row * 4] * local_origin[0]
            + matrix[row * 4 + 1] * local_origin[1]
            + matrix[row * 4 + 2] * local_origin[2]
            + matrix[row * 4 + 3]
    });
    let world_direction = [0, 1, 2].map(|row| {
        matrix[row * 4] * local_direction[0]
            + matrix[row * 4 + 1] * local_direction[1]
            + matrix[row * 4 + 2] * local_direction[2]
    });
    let pattern = |axis| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CircularPattern {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![1],
            },
            instances: 2,
            axis,
            angle_step_degrees: 90.0,
        }],
    };
    let edge_plan = session
        .plan_cad_program(
            &pattern(AssistantAxisSpec::Edge {
                edge_reference_id: reference_id.clone(),
                instance_path: Some(AssistantInstancePath {
                    root_occurrence_id: 2,
                    steps: Vec::new(),
                }),
            }),
            &BTreeSet::new(),
        )
        .unwrap();
    let direct_plan = session
        .plan_cad_program(
            &pattern(AssistantAxisSpec::OriginDirection {
                origin_mm: world_origin,
                direction: world_direction,
            }),
            &BTreeSet::new(),
        )
        .unwrap();
    assert_eq!(edge_plan.batch().commands(), direct_plan.batch().commands());

    let nonuniform = session
        .plan_cad_program(
            &pattern(AssistantAxisSpec::Edge {
                edge_reference_id: reference_id.clone(),
                instance_path: Some(AssistantInstancePath {
                    root_occurrence_id: 3,
                    steps: Vec::new(),
                }),
            }),
            &BTreeSet::new(),
        )
        .unwrap_err();
    assert!(matches!(
        nonuniform,
        SessionError::Planning(error)
            if error.code == "planning.cad_axis_instance_transform_invalid"
    ));

    let ambiguous = session
        .plan_cad_program(
            &pattern(AssistantAxisSpec::Edge {
                edge_reference_id: reference_id,
                instance_path: None,
            }),
            &BTreeSet::new(),
        )
        .unwrap_err();
    assert!(matches!(
        ambiguous,
        SessionError::Planning(error)
            if error.code == "planning.cad_axis_instance_path_unavailable"
    ));
    let before = session.snapshot();
    session.apply_proposal(&edge_plan).unwrap();
    assert_eq!(session.snapshot().occurrences().count(), 4);
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        before.canonical_digest()
    );
}

#[test]
fn real_worker_query_selects_two_upper_circular_edges_for_one_fillet_operation() {
    let settings = worker_settings();
    let mut session = DocumentSession::new(settings.clone());
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::CreatePart {
                    name: "Annular part".into(),
                    workplane: AssistantWorkplaneSpec::Principal {
                        plane: AssistantPrincipalPlane::Xy,
                    },
                    entities: vec![
                        AssistantSketchEntity::Circle {
                            id: 1,
                            center_mm: [12.0, -7.0],
                            radius_mm: 10.0,
                        },
                        AssistantSketchEntity::Circle {
                            id: 2,
                            center_mm: [12.0, -7.0],
                            radius_mm: 6.0,
                        },
                    ],
                    constraints: vec![
                        AssistantSketchConstraint::Radius {
                            id: 1,
                            entity_id: 1,
                            value_mm: 10.0,
                        },
                        AssistantSketchConstraint::Radius {
                            id: 2,
                            entity_id: 2,
                            value_mm: 6.0,
                        },
                    ],
                    feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
                    translation_mm: [0.0, 0.0, 0.0],
                    rotation: None,
                }],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let created = session.snapshot();
    let report = session.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");

    let query = ModelQuery::default();
    let page = query
        .page_with_topology(
            &created,
            session.topology_results(),
            &PageRequest {
                kind: EntityKind::Edges,
                limit: 100,
                search: "circle".into(),
                definition_id: Some(1),
                tag_id: None,
                classification_dimension_id: None,
                classification_category_id: None,
                world_bounds_mm: None,
                cursor: None,
            },
        )
        .unwrap();
    let queried_edges = page["items"].as_array().unwrap();
    assert!(
        queried_edges
            .iter()
            .all(|edge| edge["id"].as_u64().is_some_and(|id| id > 0))
    );
    let queried_edge = queried_edges
        .iter()
        .find(|edge| edge["geometry"]["circle_radius_mm"] == 6.0)
        .unwrap();
    let detail = query
        .detail_with_topology(
            &created,
            session.topology_results(),
            EntityKind::Edges,
            queried_edge["id"].as_u64().unwrap(),
        )
        .unwrap();
    assert_eq!(detail["item"], *queried_edge);
    assert_eq!(detail["identity"], page["identity"]);
    assert_eq!(detail["completeness"]["metadata_only"], false);

    let mut upper_edges = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["producer_feature_id"] == 3
                && edge["geometry"]["closed"] == true
                && edge["geometry"]["centroid_mm"][2]
                    .as_f64()
                    .is_some_and(|z| (z - 30.0).abs() <= 1.0e-9)
                && edge["geometry"]["unit_axis_direction"][2]
                    .as_f64()
                    .is_some_and(|z| z.abs() >= 1.0 - 1.0e-12)
        })
        .map(|edge| {
            (
                edge["geometry"]["circle_radius_mm"].as_f64().unwrap(),
                edge["reference_id"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    upper_edges.sort_by(|left, right| left.0.total_cmp(&right.0));
    assert_eq!(
        upper_edges
            .iter()
            .map(|(radius, _)| *radius)
            .collect::<Vec<_>>(),
        vec![6.0, 10.0]
    );

    let undo = session.visible_undo_steps();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::FilletEdges {
                    definition_id: 1,
                    name: "Upper rim fillet".into(),
                    target_feature_id: 3,
                    edge_reference_ids: upper_edges
                        .into_iter()
                        .map(|(_, reference_id)| reference_id)
                        .collect(),
                    radius_mm: 1.0,
                }],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let filleted = session.snapshot();
    assert_eq!(session.visible_undo_steps(), undo + 1);
    assert!(matches!(
        filleted.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::TopologyEdgeFinish {
            target: FeatureId(3),
            kind: EdgeFinishKind::Fillet,
            edges,
            amount,
        } if edges.len() == 2 && amount.millimetres() == 1.0
    ));
    let report = session.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
    assert!(session.topology_results().values().any(|package| {
        matches!(
            package.as_ref(),
            ketchup_core::exact_product::ExactBodyPackage::Graph(graph)
                if graph.identity.producer_feature_id == FeatureId(4)
                    && graph.volume_mm3 > 0.0
        )
    }));

    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        created.canonical_digest()
    );
    assert!(session.snapshot().feature(FeatureId(4)).is_none());
    let report = session.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");

    session.redo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        filleted.canonical_digest()
    );
    let report = session.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("annular-fillet.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    let mut reopened = DocumentSession::open(&path, settings).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        filleted.canonical_digest()
    );
    let report = reopened.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
    reopened.undo().unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        created.canonical_digest()
    );
    reopened.redo().unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        filleted.canonical_digest()
    );
}

#[test]
fn missing_worker_empty_coverage_and_invalid_inputs_are_honest() {
    let mut session = DocumentSession::new(SessionSettings {
        exact_worker_path: Some("nonexistent-worker".into()),
        ..SessionSettings::default()
    });
    let report = session.evaluate().unwrap();
    assert!(!report.complete);
    assert!(report.not_evaluated.is_some());
    let invalid = AssistantCadEditProgram { operations: vec![] };
    assert!(matches!(
        session.plan_cad_program(&invalid, &BTreeSet::new()),
        Err(SessionError::Planning(_))
    ));
    assert!(
        serde_json::from_str::<AssistantCadEditProgram>(
            r#"{"operations":[{"operation":"does_not_exist"}]}"#
        )
        .is_err()
    );
    session
        .apply_cad_program(&program(), &BTreeSet::new())
        .unwrap();
    let report = session.evaluate().unwrap();
    assert!(!report.complete);
    assert_eq!(report.producers.len(), 1);
    assert!(matches!(
        report.producers[0].render,
        EvidenceStatus::NotEvaluated { .. }
    ));
    let validation = session.validators(&AssistantValidationSelection::only(&["bogus"]));
    assert_eq!(validation["state"], "not_evaluated");
    assert_eq!(
        session.validators(&AssistantValidationSelection::only(&[]))["complete"],
        false
    );
    assert_eq!(session.visible_undo_steps(), 1);
}
#[test]
fn shared_poll_wait_cancel_and_stale_publication() {
    let settings = worker_settings();
    let mut document = DocumentStore::new();
    let batch = ketchup_application::plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let mut render = ExactResultRegistry::default();
    let mut topology = ExactResultRegistry::default();
    let task = start_exact_evaluation(
        document.current(),
        &ContainerData::default(),
        &render,
        &topology,
        settings.exact_worker_path.clone(),
        || {},
    );
    assert_eq!(task.progress().total_producers, 1);
    assert_eq!(task.progress().reused_producers, 0);
    let products = task.wait(Duration::from_secs(30)).unwrap();
    assert_eq!(task.progress().completed_producers, 1);
    let report =
        publish_exact_products(&mut document, &mut render, &mut topology, &task, products).unwrap();
    assert!(report.complete);
    let task = start_exact_evaluation(
        document.current(),
        &ContainerData::default(),
        &render,
        &topology,
        settings.exact_worker_path.clone(),
        || {},
    );
    assert_eq!(
        task.progress(),
        ketchup_application::evaluation::ExactEvaluationProgress {
            total_producers: 1,
            completed_producers: 1,
            reused_producers: 1,
            active_producer: None,
            active_elapsed_ms: None,
        }
    );
    let products = loop {
        match task.poll() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("{error}"),
        }
    };
    document.undo().unwrap();
    assert!(
        publish_exact_products(&mut document, &mut render, &mut topology, &task, products).is_err()
    );
    let task = start_exact_evaluation(
        document.current(),
        &ContainerData::default(),
        &render,
        &topology,
        settings.exact_worker_path,
        || {},
    );
    task.cancel();
    assert!(task.is_cancelled());
    assert!(task.wait(Duration::from_secs(1)).is_err());
}

use ketchup_core::sketch::*;
fn rectangle_sketch(workplane: FeatureId, min_mm: [f64; 2], max_mm: [f64; 2]) -> SketchSpec {
    let corners = [
        min_mm,
        [max_mm[0], min_mm[1]],
        max_mm,
        [min_mm[0], max_mm[1]],
    ];
    let entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % corners.len()],
        })
        .collect::<Vec<_>>();
    let mut constraints = Vec::new();
    for index in 0..4 {
        let entity = SketchEntityId(index as u64 + 1);
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 1),
            kind: SketchConstraintKind::FixedPoint {
                point: SketchPointRef {
                    entity,
                    point: SketchPointKind::Start,
                },
                position_mm: corners[index],
            },
        });
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 2),
            kind: SketchConstraintKind::FixedPoint {
                point: SketchPointRef {
                    entity,
                    point: SketchPointKind::End,
                },
                position_mm: corners[(index + 1) % corners.len()],
            },
        });
    }
    SketchSpec {
        workplane,
        entities,
        constraints,
    }
}

fn apply_session_batch(
    session: &mut DocumentSession,
    batch: &CommandBatch,
) -> Result<Snapshot, SessionError> {
    let proposal = session.plan_commands(batch.clone())?;
    session.apply_proposal(&proposal)
}
#[test]
fn real_worker_face_supported_pocket_keeps_intermediate_and_roundtrips() {
    let definition = DefinitionId(2);
    let base_plane = FeatureId(20);
    let base_sketch_id = FeatureId(21);
    let pad = FeatureId(22);
    let face_plane = FeatureId(23);
    let pocket_sketch_id = FeatureId(24);
    let pocket = FeatureId(25);
    let base_sketch = rectangle_sketch(base_plane, [10.0, 20.0], [110.0, 80.0]);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentSession::new(worker_settings());
    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Pad Pocket".into(),
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
                name: "Base rectangle".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: definition,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: base_sketch_id,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("18").unwrap()),
                }),
            },
        ]),
    )
    .unwrap();

    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(1),
            definition_id: definition,
            name: "Pad".into(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }]),
    )
    .unwrap();
    assert!(document.evaluate().unwrap().complete);
    let package = document.exact_results().values().next().unwrap();
    let ketchup_core::exact_product::ExactBodyPackage::Rectangle(pad_package) = package.as_ref()
    else {
        panic!("expected rectangle fast path")
    };
    let top = pad_package
        .reference(ketchup_core::exact_product::ExactFaceRole::Top)
        .unwrap()
        .clone();
    let pocket_sketch = rectangle_sketch(face_plane, [30.0, 20.0], [50.0, 35.0]);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: face_plane,
                definition_id: definition,
                name: "Pad top".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [10.0, 20.0, 18.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: pocket_sketch_id,
                definition_id: definition,
                name: "Pocket rectangle".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
        ]),
    )
    .unwrap();
    apply_session_batch(
        &mut document,
        &CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: pocket,
            definition_id: definition,
            name: "Pocket".into(),
            kind: FeatureKind::SketchPocket(PocketSpec {
                target: pad,
                sketch: pocket_sketch_id,
                region: pocket_region,
                support: Box::new(top),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal("6").unwrap()),
            }),
        }]),
    )
    .unwrap();
    let before = document.snapshot();
    let undo = document.visible_undo_steps();
    let report = document.evaluate().unwrap();
    assert!(report.complete, "{report:?}");
    assert!(
        report
            .producers
            .iter()
            .any(|entry| entry.key.feature_id == pocket)
    );
    assert!(
        report
            .producers
            .iter()
            .any(|entry| entry.key.feature_id == pad)
    );
    assert_eq!(document.visible_undo_steps(), undo);
    assert_eq!(
        document.snapshot().canonical_digest(),
        before.canonical_digest()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pocket.ketchup");
    document.save(&path, SaveOptions::default()).unwrap();
    let mut reopened = DocumentSession::open(&path, worker_settings()).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        before.canonical_digest()
    );
    assert!(reopened.evaluate().unwrap().complete);
}

#[test]
fn typed_cad_program_commits_static_metadata_atomically() {
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![part(), part()],
            },
            &BTreeSet::new(),
        )
        .unwrap();
    let geometry = session.snapshot();
    let metadata = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::UpsertClassificationDimension {
                dimension_id: 1,
                name: "ketchup.validator-role.v1".into(),
                categories: vec![
                    AssistantCadClassificationCategory {
                        id: 1,
                        name: "physics.static.load:case-a".into(),
                    },
                    AssistantCadClassificationCategory {
                        id: 2,
                        name: "physics.static.support:case-a".into(),
                    },
                ],
            },
            AssistantCadEditOperation::SetOccurrenceClassification {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: vec![1],
                },
                dimension_id: 1,
                category_id: Some(1),
            },
            AssistantCadEditOperation::SetOccurrenceClassification {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: vec![2],
                },
                dimension_id: 1,
                category_id: Some(2),
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 1,
                name: "physics.gravity_x_m_s2".into(),
                value: 0.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 2,
                name: "physics.gravity_y_m_s2".into(),
                value: 0.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 3,
                name: "physics.gravity_z_m_s2".into(),
                value: -9.81,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 4,
                name: "physics.mass_kg.occurrence.1".into(),
                value: 100.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 5,
                name: "physics.applied_load_n.occurrence.1".into(),
                value: 200.0,
            },
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id: 6,
                name: "physics.support_capacity_n.occurrence.2".into(),
                value: 2_000.0,
            },
        ],
    };

    let proposal = session
        .plan_cad_program(&metadata, &BTreeSet::new())
        .unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        geometry.canonical_digest()
    );
    assert_eq!(session.visible_undo_steps(), 1);
    let committed = session.apply_proposal(&proposal).unwrap();
    assert_eq!(session.visible_undo_steps(), 2);
    assert_eq!(
        committed.occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(1))
    );
    let scope = StructuralValidationScope::bind(&committed, [OccurrenceId(1)]);
    let report = scoped_static_load_report(&committed, &scope, || false);
    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["coverage"]["checked_load_occurrence_count"], 1);

    session.undo().unwrap();
    assert_eq!(
        session.snapshot().canonical_digest(),
        geometry.canonical_digest()
    );
}
