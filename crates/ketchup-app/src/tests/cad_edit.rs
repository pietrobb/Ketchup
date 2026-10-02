//! Assistant CAD edit programs: host IDs, exact preflight, one Undo step.

use super::*;

#[test]
fn cad_edit_program_compiles_selection_to_one_host_id_canonical_batch() {
    let mut app = KetchupApp::new();
    app.selection.occurrences = BTreeSet::from([InstancePath::root(OccurrenceId(1))]);
    let selector = AssistantCadEntitySelector::CurrentSelection {};
    let program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::Transform {
                selector: selector.clone(),
                translation_mm: [10.0, 0.0, 0.0],
                rotation: Some(ketchup_assistant::sidecar::AssistantCadRotation {
                    pivot_mm: [0.0, 0.0, 0.0],
                    axis: [0.0, 0.0, 1.0],
                    angle_degrees: 90.0,
                }),
            },
            AssistantCadEditOperation::Copy {
                selector: selector.clone(),
                translation_mm: [0.0, 20.0, 0.0],
            },
            AssistantCadEditOperation::LinearPattern {
                selector: selector.clone(),
                instances: 3,
                step_mm: [25.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::Mirror {
                selector,
                plane_origin_mm: [0.0, 0.0, 0.0],
                plane_normal: [1.0, 0.0, 0.0],
            },
        ],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert_eq!(batch.commands().len(), 5);
    assert!(matches!(
        batch.commands()[0],
        CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(1),
            ..
        }
    ));
    assert_eq!(
        batch
            .commands()
            .iter()
            .filter_map(|command| match command {
                CanonicalCommand::CreateOccurrence { id, .. } => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            OccurrenceId(2),
            OccurrenceId(3),
            OccurrenceId(4),
            OccurrenceId(5)
        ]
    );

    app.document.apply_batch(&batch).unwrap();
    assert_eq!(app.occurrence_count(), 5);
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform(),
        match batch.commands()[0] {
            CanonicalCommand::SetOccurrenceTransform { transform, .. } => transform,
            _ => unreachable!(),
        }
    );
}

#[test]
fn cad_edit_append_boolean_is_host_id_assigned_exact_and_one_step() {
    for (assistant_operation, canonical_operation) in [
        (AssistantCadBooleanOperation::Cut, BooleanOperation::Cut),
        (AssistantCadBooleanOperation::Union, BooleanOperation::Union),
        (
            AssistantCadBooleanOperation::Intersect,
            BooleanOperation::Intersect,
        ),
    ] {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: INITIAL_BOX_DEFINITION,
                    name: "Tool profile".to_owned(),
                    kind: FeatureKind::polygon(&[
                        [40.0, 0.0],
                        [120.0, 0.0],
                        [120.0, 60.0],
                        [40.0, 60.0],
                    ]),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: INITIAL_BOX_DEFINITION,
                    name: "Tool extrusion".to_owned(),
                    kind: FeatureKind::extrusion(FeatureId(3), Dimension::new("20", 20.0).unwrap()),
                },
            ]))
            .unwrap();
        let baseline = app.document.current().clone();
        let baseline_undo = app.document.visible_undo_steps();
        let program = AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: INITIAL_BOX_DEFINITION.0.into(),
                name: "Assistant Boolean".to_owned(),
                feature: AssistantCadBodyFeature::Boolean {
                    operation: assistant_operation,
                    target_feature_id: 2.into(),
                    tool_feature_id: 4.into(),
                },
            }],
        };

        let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
        assert!(matches!(
            batch.commands(),
            [CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: INITIAL_BOX_DEFINITION,
                kind: FeatureKind::Boolean {
                    operation,
                    target: FeatureId(2),
                    tool: FeatureId(4),
                },
                ..
            }] if operation == &canonical_operation
        ));

        app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
            .unwrap();
        assert_eq!(app.document.current().revision_id(), baseline.revision_id());
        assert_eq!(
            app.document.current().canonical_digest(),
            baseline.canonical_digest()
        );
        assert_eq!(app.document.visible_undo_steps(), baseline_undo);
        assert!(app.confirm_assistant_proposal());
        let committed = app.document.current();
        assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
        assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
        assert!(
            ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(5)).is_ok()
        );
        assert!(app.undo());
        assert_eq!(
            app.document.current().canonical_digest(),
            baseline.canonical_digest()
        );
    }
}

#[test]
fn cad_edit_append_pocket_is_host_id_assigned_exact_and_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(3),
            definition_id: INITIAL_BOX_DEFINITION,
            name: "Pocket profile".to_owned(),
            kind: FeatureKind::polygon(&[[20.0, 15.0], [40.0, 15.0], [40.0, 35.0], [20.0, 35.0]]),
        }]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Assistant pocket".to_owned(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2.into(),
                profile_feature_id: 3.into(),
                depth_mm: 8.0,
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(4),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::Pad(PadSpec {
                profile: PadProfile::Feature(FeatureId(3)),
                extent: FeatureExtent::Blind(depth),
                operation: PadOperation::Cut { target: FeatureId(2), .. },
                ..
            }),
            ..
        }] if depth.millimetres() == 8.0
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(4)).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::ProfileCut { .. }))
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_append_planar_offset_is_host_id_assigned_exact_and_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Offset profile".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(2),
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[
                    [10.0, 20.0],
                    [90.0, 20.0],
                    [90.0, 60.0],
                    [10.0, 60.0],
                ]),
            },
        ]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 2.into(),
            name: "Assistant planar offset".to_owned(),
            feature: AssistantCadBodyFeature::PlanarOffset {
                profile_feature_id: 3,
                distance_mm: -5.0,
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(4),
            definition_id: DefinitionId(2),
            kind: FeatureKind::PlanarOffset { profile: FeatureId(3), distance },
            ..
        }] if distance.millimetres() == -5.0
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph = ExactBRepGraph::from_snapshot(&committed, DefinitionId(2), FeatureId(4)).unwrap();
    assert_eq!(graph.profiles[0].source_feature_id, 3);
    assert!(matches!(
        graph.nodes.last().unwrap().operation,
        ExactBRepOperation::PlanarOffset { distance_bits, .. }
            if f64::from_bits(distance_bits) == -5.0
    ));
    assert_eq!(
        graph.producer_bounds_mm().unwrap(),
        Some([[15.0, 25.0, 0.0], [85.0, 55.0, 0.0]])
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_append_sweep_is_host_id_assigned_exact_and_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep profile".to_owned(),
                kind: FeatureKind::polygon(&[[-2.0, -3.0], [2.0, -3.0], [2.0, 3.0], [-2.0, 3.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep path".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [10.0, -5.0],
                        end_mm: [24.0, 17.0],
                    }],
                    closed: false,
                },
            },
        ]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Assistant sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: 3,
                path_feature_id: 4,
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(5),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::Sweep {
                profile: FeatureId(3),
                path: FeatureId(4),
                ..
            },
            ..
        }]
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(5)).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::Sweep { .. }))
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_append_spatial_sweep_preflights_v12_without_mutation_and_is_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Spatial sweep profile".to_owned(),
                kind: FeatureKind::polygon(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Non-coplanar mixed path".to_owned(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![
                        SpatialPathSegment::Line {
                            start_mm: [0.0, 0.0, 0.0],
                            end_mm: [30.0, 0.0, 0.0],
                        },
                        SpatialPathSegment::CircularArc {
                            start_mm: [30.0, 0.0, 0.0],
                            end_mm: [40.0, 10.0, 0.0],
                            center_mm: [30.0, 10.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                            clockwise: false,
                        },
                        SpatialPathSegment::CubicBezier {
                            start_mm: [40.0, 10.0, 0.0],
                            control_1_mm: [40.0, 20.0, 0.0],
                            control_2_mm: [40.0, 30.0, 10.0],
                            end_mm: [40.0, 40.0, 20.0],
                        },
                    ],
                },
            },
        ]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Assistant spatial sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: 3,
                path_feature_id: 4,
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(5),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::Sweep {
                profile: FeatureId(3),
                path: FeatureId(4),
                ..
            },
            ..
        }]
    ));
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);

    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    let committed_digest = committed.canonical_digest();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(5)).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V12);
    assert!(matches!(
        graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { .. }
    ));

    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.redo());
    assert_eq!(app.document.current().canonical_digest(), committed_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
}

#[test]
fn cad_edit_append_spatial_sweep_rejects_combined_envelope_without_mutation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Bounded sweep profile".to_owned(),
                kind: FeatureKind::polygon(&[[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Individually bounded spatial path".to_owned(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [999_999.5, 0.0, 0.0],
                        end_mm: [999_999.5, 0.0, 10.0],
                    }],
                },
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected spatial sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: 3,
                path_feature_id: 4,
            },
        }],
    };

    let error = app.plan_assistant_cad_edit_program(&program).unwrap_err();
    assert_eq!(error.code, "canonical.invalid_sweep");
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_spatial_sweep_rejects_suppressed_and_cross_definition_paths_atomically() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Spatial sweep definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(2),
                name: "Spatial sweep profile".to_owned(),
                kind: FeatureKind::polygon(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Valid spatial path".to_owned(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 20.0],
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(2),
                name: "Existing spatial sweep".to_owned(),
                kind: FeatureKind::sweep(FeatureId(3), FeatureId(4)),
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(3),
                name: "Other spatial path definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(3),
                name: "Valid cross-definition spatial path".to_owned(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [20.0, 0.0, 0.0],
                    }],
                },
            },
        ]))
        .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id: DefinitionId(2),
                body_id: BodyId(1),
                suppressed_feature_ids: vec![FeatureId(4), FeatureId(5)],
            },
        ]))
        .unwrap();
    assert!(app.document.current().feature_is_suppressed(FeatureId(4)));

    let baseline = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
        app.document.visible_redo_steps(),
    );
    let program = |path_feature_id| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 2.into(),
            name: "Rejected spatial sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: 3,
                path_feature_id,
            },
        }],
    };

    let suppressed = app
        .plan_assistant_cad_edit_program(&program(4))
        .unwrap_err();
    assert_eq!(suppressed.code, "canonical.invalid_feature_suppression");
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
            app.document.visible_redo_steps(),
        ),
        baseline
    );

    let cross_definition = app
        .plan_assistant_cad_edit_program(&program(6))
        .unwrap_err();
    assert_eq!(
        cross_definition.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
            app.document.visible_redo_steps(),
        ),
        baseline
    );
}

#[test]
fn cad_edit_append_loft_is_host_id_assigned_exact_and_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Lower spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-8.0, -4.0],
                    [9.0, -3.0],
                    [7.0, 6.0],
                    [-6.0, 5.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Upper spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-4.0, -2.0],
                    [5.0, -2.0],
                    [4.0, 3.0],
                    [-3.0, 4.0],
                ]),
            },
        ]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Assistant loft".to_owned(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: 3.into(),
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: 4.into(),
                        elevation_mm: 35.0,
                    },
                ],
                guide_feature_id: None,
                continuity: AssistantCadLoftContinuity::Position,
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(5),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::Loft { sections, .. },
            ..
        }] if sections == &vec![
            LoftSection { profile: FeatureId(3), elevation_mm: 0.0 },
            LoftSection { profile: FeatureId(4), elevation_mm: 35.0 },
        ]
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(5)).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::Loft { .. }))
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_append_topology_shell_uses_host_face_reference_and_one_step() {
    let mut app = KetchupApp::new();
    let body_id = app
        .document
        .current()
        .definition(INITIAL_BOX_DEFINITION)
        .unwrap()
        .bodies()
        .next()
        .unwrap()
        .id();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyVisibility {
                definition_id: INITIAL_BOX_DEFINITION,
                id: body_id,
                visible: false,
            },
        ]))
        .unwrap();
    install_initial_graph_result(&mut app);
    let context = app.assistant_context_for("Shell the exact body");
    assert_eq!(context["topology_face_references_complete"], true);
    let references = context["topology_face_references"].as_array().unwrap();
    let expected_reference_ids = references
        .iter()
        .filter(|reference| reference["target_feature_id"] == 2)
        .take(2)
        .map(|reference| reference["reference_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(expected_reference_ids.len(), 2);
    let mut requested_reference_ids = expected_reference_ids.clone();
    requested_reference_ids.reverse();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Assistant shell".to_owned(),
            feature: AssistantCadBodyFeature::TopologyShell {
                target_feature_id: 2,
                removed_face_reference_ids: requested_reference_ids,
                thickness_mm: 2.0,
                direction: ketchup_assistant::sidecar::AssistantCadShellDirection::Inward,
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(3),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::Shell {
                target: FeatureId(2),
                removed_faces,
                thickness,
                direction: ketchup_model::document::ShellDirection::Inward,
                ..
            },
            ..
        }] if removed_faces.len() == 2
            && removed_faces.iter().filter_map(FaceRef::topological).all(|reference| {
                reference.producer_feature_id == FeatureId(2)
                    && reference.kind == TopologicalElementKind::Face
            })
            && removed_faces
                .iter()
                .filter_map(FaceRef::topological)
                .map(|reference| reference.lineage_digest.clone())
                .collect::<Vec<_>>() == expected_reference_ids
            && thickness.millimetres() == 2.0
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(3)).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::Shell { .. }))
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_fillet_edges_uses_host_references_and_one_step() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let context = app.assistant_context_for("Fillet the exact body edges");
    assert_eq!(context["topology_edge_references_complete"], true);
    let references = context["topology_edge_references"].as_array().unwrap();
    let expected_reference_ids = references
        .iter()
        .filter(|reference| reference["target_feature_id"] == 2)
        .take(2)
        .map(|reference| reference["reference_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(expected_reference_ids.len(), 2);
    let mut requested_reference_ids = expected_reference_ids.clone();
    requested_reference_ids.reverse();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::FilletEdges {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Assistant fillet".to_owned(),
            target_feature_id: 2,
            edge_reference_ids: requested_reference_ids,
            radius_mm: 2.0,
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(3),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::EdgeFinish {
                target: FeatureId(2),
                edges,
                kind: EdgeFinishKind::Fillet,
                amount,
                ..
            },
            ..
        }] if edges.len() == 2
            && edges.iter().filter_map(EdgeRef::topological).all(|reference| {
                reference.producer_feature_id == FeatureId(2)
                    && reference.kind == TopologicalElementKind::Edge
            })
            && edges
                .iter()
                .filter_map(EdgeRef::topological)
                .map(|reference| reference.lineage_digest.clone())
                .collect::<Vec<_>>() == expected_reference_ids
            && amount.millimetres() == 2.0
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(3)).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::EdgeFinish { .. }))
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_chamfer_edges_uses_host_references_and_one_step() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let context = app.assistant_context_for("Chamfer an exact body edge");
    let expected_reference_ids = context["topology_edge_references"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reference| reference["target_feature_id"] == 2)
        .take(2)
        .map(|reference| reference["reference_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(expected_reference_ids.len(), 2);
    let mut requested_reference_ids = expected_reference_ids.clone();
    requested_reference_ids.reverse();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::ChamferEdges {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Assistant chamfer".to_owned(),
            target_feature_id: 2,
            edge_reference_ids: requested_reference_ids,
            distance_mm: 2.0,
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(3),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::EdgeFinish {
                target: FeatureId(2),
                edges,
                kind: EdgeFinishKind::Chamfer,
                amount,
                ..
            },
            ..
        }] if edges.len() == 2
            && edges
                .iter()
                .filter_map(EdgeRef::topological)
                .map(|reference| reference.lineage_digest.clone())
                .collect::<Vec<_>>() == expected_reference_ids
            && amount.millimetres() == 2.0
    ));

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
        .unwrap();
    assert_eq!(app.document.current().revision_id(), baseline.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    assert!(app.confirm_assistant_proposal());
    let committed = app.document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo + 1);
    let graph =
        ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(3)).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::EdgeFinish { .. }))
    );
    assert!(app.undo());
    assert_eq!(
        app.document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn cad_edit_append_topology_shell_rejects_unpublished_reference_without_mutation() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected shell".to_owned(),
            feature: AssistantCadBodyFeature::TopologyShell {
                target_feature_id: 2,
                removed_face_reference_ids: vec!["f".repeat(64)],
                thickness_mm: 2.0,
                direction: ketchup_assistant::sidecar::AssistantCadShellDirection::Inward,
            },
        }],
    };

    let error = app.plan_assistant_cad_edit_program(&program).unwrap_err();
    assert_eq!(error.code, "planning.cad_topology_reference_unavailable");
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_topology_fillet_rejects_unpublished_reference_without_mutation() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected fillet".to_owned(),
            feature: AssistantCadBodyFeature::TopologyFillet {
                target_feature_id: 2,
                edge_reference_ids: vec!["f".repeat(64)],
                radius_mm: 2.0,
                radius_stations: Vec::new(),
            },
        }],
    };

    let error = app.plan_assistant_cad_edit_program(&program).unwrap_err();
    assert_eq!(error.code, "planning.cad_topology_reference_unavailable");
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_topology_chamfer_rejects_unpublished_reference_without_mutation() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected chamfer".to_owned(),
            feature: AssistantCadBodyFeature::TopologyChamfer {
                target_feature_id: 2,
                edge_reference_ids: vec!["f".repeat(64)],
                distance_mm: 2.0,
                mode: Default::default(),
                side_face_reference_ids: Vec::new(),
            },
        }],
    };

    let error = app.plan_assistant_cad_edit_program(&program).unwrap_err();
    assert_eq!(error.code, "planning.cad_topology_reference_unavailable");
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_pocket_rejects_invalid_inputs_without_mutation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Pocket profile".to_owned(),
                kind: FeatureKind::polygon(&[
                    [20.0, 15.0],
                    [40.0, 15.0],
                    [40.0, 35.0],
                    [20.0, 35.0],
                ]),
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Other profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]]),
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |target_feature_id, profile_feature_id, depth_mm| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected pocket".to_owned(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id,
                profile_feature_id,
                depth_mm,
            },
        }],
    };

    assert!(
        app.plan_assistant_cad_edit_program(&program(2.into(), 4.into(), 8.0))
            .is_err()
    );
    assert!(
        app.plan_assistant_cad_edit_program(&program(2.into(), 3.into(), 20.0))
            .is_err()
    );
    assert!(
        app.plan_assistant_cad_edit_program(&program(3.into(), 2.into(), 8.0))
            .is_err()
    );
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_planar_offset_rejects_unsupported_inputs_without_mutation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |definition_id: u64, profile_feature_id, distance_mm| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: definition_id.into(),
            name: "Rejected planar offset".to_owned(),
            feature: AssistantCadBodyFeature::PlanarOffset {
                profile_feature_id,
                distance_mm,
            },
        }],
    };

    let extra_feature_error = app
        .plan_assistant_cad_edit_program(&program(1, 1, 5.0))
        .unwrap_err();
    assert_eq!(
        extra_feature_error.code,
        "planning.cad_feature_input_unsupported"
    );
    let ownership_error = app
        .plan_assistant_cad_edit_program(&program(2, 1, 5.0))
        .unwrap_err();
    assert_eq!(
        ownership_error.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    assert!(
        app.plan_assistant_cad_edit_program(&program(2, 999, 5.0))
            .is_err()
    );
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);

    for (points_mm, distance_mm, expected_code) in [
        // Inset past its 16.6 mm inradius, the triangle would vanish.
        (
            vec![[0.0, 0.0], [80.0, 0.0], [40.0, 40.0]],
            -20.0,
            "canonical.invalid_planar_offset",
        ),
        (
            vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
            -20.0,
            "canonical.invalid_planar_offset",
        ),
    ] {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Offset profile".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(2),
                    name: "Invalid offset profile".to_owned(),
                    kind: FeatureKind::polygon(&points_mm),
                },
            ]))
            .unwrap();
        let baseline_revision = app.document.current().revision_id();
        let baseline_digest = app.document.current().canonical_digest();
        let baseline_undo = app.document.visible_undo_steps();
        let error = app
            .plan_assistant_cad_edit_program(&program(2, 3, distance_mm))
            .unwrap_err();
        assert_eq!(error.code, expected_code);
        assert_eq!(app.document.current().revision_id(), baseline_revision);
        assert_eq!(app.document.current().canonical_digest(), baseline_digest);
        assert_eq!(app.document.visible_undo_steps(), baseline_undo);
    }
}

#[test]
fn cad_edit_append_sweep_rejects_unsupported_inputs_without_mutation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep profile".to_owned(),
                kind: FeatureKind::polygon(&[[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep path".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [20.0, 0.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Unsupported spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-3.0, -2.0],
                    [4.0, -2.0],
                    [4.0, 3.0],
                    [-3.0, 3.0],
                ]),
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(2),
                name: "Other profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(8),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Overlong path".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [-600_000.0, 0.0],
                        end_mm: [600_000.0, 0.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(9),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sub-minimum arc".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [0.001, 0.0],
                            end_mm: [-0.001, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                        ProfileSegment::Line {
                            start_mm: [-0.001, 0.0],
                            end_mm: [0.001, 0.0],
                        },
                    ],
                    closed: true,
                },
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |profile_feature_id, path_feature_id| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id,
                path_feature_id,
            },
        }],
    };

    let cross_definition = app
        .plan_assistant_cad_edit_program(&program(6, 4))
        .unwrap_err();
    assert_eq!(
        cross_definition.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    let spline = app
        .plan_assistant_cad_edit_program(&program(5, 4))
        .unwrap_err();
    assert_eq!(spline.code, "canonical.invalid_sweep");
    let invalid_path = app
        .plan_assistant_cad_edit_program(&program(3, 1))
        .unwrap_err();
    assert_eq!(invalid_path.code, "canonical.invalid_sweep");
    // A line loop enclosing no area is refused as a profile, before any sweep.
    assert_eq!(
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FeatureId(7),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Zero-area boundary".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0]]),
            }]))
            .err(),
        Some(CanonicalError::InvalidProfile)
    );
    let overlong_path = app
        .plan_assistant_cad_edit_program(&program(3, 8))
        .unwrap_err();
    assert_eq!(overlong_path.code, "canonical.invalid_sweep");
    let sub_minimum_arc = app
        .plan_assistant_cad_edit_program(&program(9, 4))
        .unwrap_err();
    assert_eq!(sub_minimum_arc.code, "canonical.invalid_sweep");
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_loft_rejects_unsupported_inputs_without_mutation() {
    let mut app = KetchupApp::new();
    let overlong_spline = (0..65)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / 65.0;
            [10.0 * angle.cos(), 10.0 * angle.sin()]
        })
        .collect::<Vec<_>>();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Valid spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-8.0, -4.0],
                    [9.0, -3.0],
                    [7.0, 6.0],
                    [-6.0, 5.0],
                ]),
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Other spline".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-4.0, -2.0],
                    [5.0, -2.0],
                    [4.0, 3.0],
                    [-3.0, 4.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Polygon profile".to_owned(),
                kind: FeatureKind::polygon(&[[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Overlong spline".to_owned(),
                kind: FeatureKind::closed_spline(&overlong_spline),
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |upper_profile_feature_id: u64| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected loft".to_owned(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: 3.into(),
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: upper_profile_feature_id.into(),
                        elevation_mm: 35.0,
                    },
                ],
                guide_feature_id: None,
                continuity: AssistantCadLoftContinuity::Position,
            },
        }],
    };

    let missing = app
        .plan_assistant_cad_edit_program(&program(99))
        .unwrap_err();
    assert_eq!(missing.code, "canonical.feature_not_found");
    let cross_definition = app
        .plan_assistant_cad_edit_program(&program(4))
        .unwrap_err();
    assert_eq!(
        cross_definition.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    for unsupported in [5, 6] {
        let rejection = app
            .plan_assistant_cad_edit_program(&program(unsupported))
            .unwrap_err();
        assert_eq!(rejection.code, "planning.cad_feature_input_unsupported");
    }
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_append_boolean_rejects_invalid_exact_inputs_without_mutation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Disjoint profile".to_owned(),
                kind: FeatureKind::polygon(&[
                    [200.0, 0.0],
                    [220.0, 0.0],
                    [220.0, 20.0],
                    [200.0, 20.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Disjoint extrusion".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(3), Dimension::new("20", 20.0).unwrap()),
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(2),
                name: "Other profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(2),
                name: "Other extrusion".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(5), Dimension::new("5", 5.0).unwrap()),
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |operation, target_feature_id, tool_feature_id| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0.into(),
            name: "Rejected Boolean".to_owned(),
            feature: AssistantCadBodyFeature::Boolean {
                operation,
                target_feature_id,
                tool_feature_id,
            },
        }],
    };

    let cross_definition = app
        .plan_assistant_cad_edit_program(&program(
            AssistantCadBooleanOperation::Cut,
            2.into(),
            6.into(),
        ))
        .unwrap_err();
    assert_eq!(
        cross_definition.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    let non_body = app
        .plan_assistant_cad_edit_program(&program(
            AssistantCadBooleanOperation::Union,
            2.into(),
            1.into(),
        ))
        .unwrap_err();
    assert_eq!(non_body.code, "planning.cad_feature_input_unsupported");
    let disjoint_intersect = app
        .plan_assistant_cad_edit_program(&program(
            AssistantCadBooleanOperation::Intersect,
            2.into(),
            4.into(),
        ))
        .unwrap_err();
    assert_eq!(disjoint_intersect.code, "planning.cad_feature_result_empty");
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
}

#[test]
fn cad_edit_current_selection_binds_to_request_time_occurrences() {
    let explicit = AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![9],
    };
    let mut program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::Copy {
                selector: AssistantCadEntitySelector::CurrentSelection {},
                translation_mm: [10.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::Mirror {
                selector: explicit.clone(),
                plane_origin_mm: [0.0, 0.0, 0.0],
                plane_normal: [1.0, 0.0, 0.0],
            },
        ],
    };

    bind_assistant_cad_current_selection(&mut program, &[3, 4]);

    assert!(matches!(
        &program.operations[0],
        AssistantCadEditOperation::Copy {
            selector: AssistantCadEntitySelector::Occurrences { occurrence_ids },
            ..
        } if occurrence_ids == &[3, 4]
    ));
    assert!(matches!(
        &program.operations[1],
        AssistantCadEditOperation::Mirror { selector, .. } if selector == &explicit
    ));
}

#[test]
fn cad_edit_program_enters_revision_bound_preview_without_mutating_document() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current().clone();
    let undo_steps = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Copy {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![1],
            },
            translation_mm: [10.0, 0.0, 0.0],
        }],
    };

    app.prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program.clone()))
        .unwrap();

    let preview = app.assistant.proposal.as_ref().unwrap();
    assert_eq!(preview.source, AssistantPreviewSource::CadEdit(program));
    assert_eq!(preview.document_id(), snapshot.document_id());
    assert_eq!(preview.provenance_revision(), snapshot.revision_id());
    assert_eq!(preview.provenance_digest(), snapshot.canonical_digest());
    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);
}

#[test]
fn cad_edit_delete_rejects_or_removes_canonical_references_atomically() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: CollectionId(1),
                name: "Protected selection".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: CollectionId(1),
                occurrence_ids: vec![OccurrenceId(1)],
            },
        ]))
        .unwrap();
    let selector = AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![1],
    };
    let rejected = app
        .plan_assistant_cad_edit_program(&AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: selector.clone(),
                dependency_policy: AssistantCadDeletePolicy::RejectIfReferenced,
            }],
        })
        .unwrap_err();
    assert_eq!(rejected.code, "planning.cad_delete_referenced");
    assert_eq!(app.occurrence_count(), 1);

    let batch = app
        .plan_assistant_cad_edit_program(&AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector,
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        })
        .unwrap();
    assert!(matches!(
        batch.commands(),
        [
            CanonicalCommand::SetCollectionOccurrences {
                id: CollectionId(1),
                occurrence_ids
            },
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(1)
            }
        ] if occurrence_ids.is_empty()
    ));
    app.document.apply_batch(&batch).unwrap();
    assert_eq!(app.occurrence_count(), 0);
    assert_eq!(
        app.document
            .current()
            .collection(CollectionId(1))
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
}
