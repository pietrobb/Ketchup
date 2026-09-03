use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ketchup_core::assistant_sidecar::AssistantCadLoftSection;
use ketchup_core::document::ProposalGoal;
use ketchup_core::graph::{EvaluatorNodeKind, PortSpec};

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
                rotation: Some(ketchup_core::assistant_sidecar::AssistantCadRotation {
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
                    kind: FeatureKind::Profile {
                        points_mm: vec![[40.0, 0.0], [120.0, 0.0], [120.0, 60.0], [40.0, 60.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: INITIAL_BOX_DEFINITION,
                    name: "Tool extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(3),
                        height: Dimension::new("20", 20.0).unwrap(),
                    },
                },
            ]))
            .unwrap();
        let baseline = app.document.current().clone();
        let baseline_undo = app.document.visible_undo_steps();
        let program = AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: INITIAL_BOX_DEFINITION.0,
                name: "Assistant Boolean".to_owned(),
                feature: AssistantCadBodyFeature::Boolean {
                    operation: assistant_operation,
                    target_feature_id: 2,
                    tool_feature_id: 4,
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
            kind: FeatureKind::Profile {
                points_mm: vec![[20.0, 15.0], [40.0, 15.0], [40.0, 35.0], [20.0, 35.0]],
            },
        }]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Assistant pocket".to_owned(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2,
                profile_feature_id: 3,
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
            kind: FeatureKind::Pocket {
                target: FeatureId(2),
                profile: FeatureId(3),
                depth,
            },
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
fn cad_edit_append_sweep_is_host_id_assigned_exact_and_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -3.0], [2.0, -3.0], [2.0, 3.0], [-2.0, 3.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep path".to_owned(),
                kind: FeatureKind::SegmentProfile {
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
            definition_id: INITIAL_BOX_DEFINITION.0,
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
fn cad_edit_append_loft_is_host_id_assigned_exact_and_one_step() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Lower spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -4.0], [9.0, -3.0], [7.0, 6.0], [-6.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Upper spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]],
                },
            },
        ]))
        .unwrap();
    let baseline = app.document.current().clone();
    let baseline_undo = app.document.visible_undo_steps();
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Assistant loft".to_owned(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: 3,
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: 4,
                        elevation_mm: 35.0,
                    },
                ],
            },
        }],
    };

    let batch = app.plan_assistant_cad_edit_program(&program).unwrap();
    assert!(matches!(
        batch.commands(),
        [CanonicalCommand::CreateFeature {
            id: FeatureId(5),
            definition_id: INITIAL_BOX_DEFINITION,
            kind: FeatureKind::Loft { sections },
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
fn cad_edit_append_pocket_rejects_invalid_inputs_without_mutation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Pocket profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[20.0, 15.0], [40.0, 15.0], [40.0, 35.0], [20.0, 35.0]],
                },
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Other profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]],
                },
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |target_feature_id, profile_feature_id, depth_mm| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Rejected pocket".to_owned(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id,
                profile_feature_id,
                depth_mm,
            },
        }],
    };

    assert!(
        app.plan_assistant_cad_edit_program(&program(2, 4, 8.0))
            .is_err()
    );
    assert!(
        app.plan_assistant_cad_edit_program(&program(2, 3, 20.0))
            .is_err()
    );
    assert!(
        app.plan_assistant_cad_edit_program(&program(3, 2, 8.0))
            .is_err()
    );
    assert_eq!(app.document.current().revision_id(), baseline_revision);
    assert_eq!(app.document.current().canonical_digest(), baseline_digest);
    assert_eq!(app.document.visible_undo_steps(), baseline_undo);
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sweep path".to_owned(),
                kind: FeatureKind::SegmentProfile {
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
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-3.0, -2.0], [4.0, -2.0], [4.0, 3.0], [-3.0, 3.0]],
                },
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(2),
                name: "Other profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(7),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Zero-area boundary".to_owned(),
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
                id: FeatureId(8),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Overlong path".to_owned(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [100_001.0, 0.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(9),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Sub-minimum arc".to_owned(),
                kind: FeatureKind::SegmentProfile {
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
            definition_id: INITIAL_BOX_DEFINITION.0,
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
    assert_eq!(spline.code, "planning.cad_feature_input_unsupported");
    let invalid_path = app
        .plan_assistant_cad_edit_program(&program(3, 1))
        .unwrap_err();
    assert_eq!(invalid_path.code, "planning.cad_feature_input_unsupported");
    let zero_area = app
        .plan_assistant_cad_edit_program(&program(7, 4))
        .unwrap_err();
    assert_eq!(zero_area.code, "planning.cad_feature_input_unsupported");
    let overlong_path = app
        .plan_assistant_cad_edit_program(&program(3, 8))
        .unwrap_err();
    assert_eq!(overlong_path.code, "planning.cad_feature_input_unsupported");
    let sub_minimum_arc = app
        .plan_assistant_cad_edit_program(&program(9, 4))
        .unwrap_err();
    assert_eq!(
        sub_minimum_arc.code,
        "planning.cad_feature_input_unsupported"
    );
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
        .collect();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Valid spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -4.0], [9.0, -3.0], [7.0, 6.0], [-6.0, 5.0]],
                },
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Other spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Polygon profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Overlong spline".to_owned(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: overlong_spline,
                },
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |upper_profile_feature_id| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Rejected loft".to_owned(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: 3,
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: upper_profile_feature_id,
                        elevation_mm: 35.0,
                    },
                ],
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
                kind: FeatureKind::Profile {
                    points_mm: vec![[200.0, 0.0], [220.0, 0.0], [220.0, 20.0], [200.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Disjoint extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(3),
                    height: Dimension::new("20", 20.0).unwrap(),
                },
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Other definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(2),
                name: "Other profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(2),
                name: "Other extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(5),
                    height: Dimension::new("5", 5.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let baseline_revision = app.document.current().revision_id();
    let baseline_digest = app.document.current().canonical_digest();
    let baseline_undo = app.document.visible_undo_steps();
    let program = |operation, target_feature_id, tool_feature_id| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: INITIAL_BOX_DEFINITION.0,
            name: "Rejected Boolean".to_owned(),
            feature: AssistantCadBodyFeature::Boolean {
                operation,
                target_feature_id,
                tool_feature_id,
            },
        }],
    };

    let cross_definition = app
        .plan_assistant_cad_edit_program(&program(AssistantCadBooleanOperation::Cut, 2, 6))
        .unwrap_err();
    assert_eq!(
        cross_definition.code,
        "planning.cad_feature_input_ownership_invalid"
    );
    let non_body = app
        .plan_assistant_cad_edit_program(&program(AssistantCadBooleanOperation::Union, 2, 1))
        .unwrap_err();
    assert_eq!(non_body.code, "planning.cad_feature_input_unsupported");
    let disjoint_intersect = app
        .plan_assistant_cad_edit_program(&program(AssistantCadBooleanOperation::Intersect, 2, 4))
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

    let preview = app.assistant_proposal.as_ref().unwrap();
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

#[test]
fn assistant_preview_planner_preserves_canonical_and_validator_diagnostics() {
    let app = KetchupApp::new();
    let canonical = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::Workflow(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(999),
                visible: false,
            },
        ))
        .unwrap_err();
    assert_eq!(
        canonical.phase,
        AssistantRejectionPhase::CanonicalValidation
    );
    assert_eq!(canonical.code, "canonical.occurrence_not_found");
    assert_eq!(canonical.operation, "workflow_intent");
    assert_eq!(
        canonical.target,
        format!("document:{}", app.document.current().document_id().0)
    );
    assert_eq!(canonical.failed_invariant, "occurrence 999 does not exist");
    assert!(canonical.retryable);
    assert_eq!(canonical.validate(), Ok(()));

    let invalid_model = serde_json::from_value::<AssistantModelIntent>(serde_json::json!({
        "replace_scene": false
    }))
    .unwrap();
    let intent = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::Model(invalid_model))
        .unwrap_err();
    assert_eq!(intent.phase, AssistantRejectionPhase::IntentValidation);
    assert_eq!(intent.code, "intent.model_invalid");
    assert_eq!(intent.validate(), Ok(()));

    let mesh_only_model = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: vec![AssistantBalloonTextIntent {
            name: "Legacy mesh-only text".to_owned(),
            text: "A".to_owned(),
            height_mm: 40.0,
            depth_mm: 16.0,
            stroke_width_mm: 8.0,
            letter_spacing_mm: 4.0,
            origin_mm: [0.0, 0.0, 0.0],
        }],
    };
    let editable_macro = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::Model(mesh_only_model))
        .unwrap_err();
    assert_eq!(
        editable_macro.phase,
        AssistantRejectionPhase::ProposalPlanning
    );
    assert_eq!(editable_macro.code, "planning.editable_macro_required");
    assert_eq!(editable_macro.operation, "model_intent");
    assert_eq!(
        editable_macro.target,
        format!("document:{}", app.document.current().document_id().0)
    );
    assert!(
        editable_macro
            .failed_invariant
            .contains("non-editable mesh")
    );
    assert!(editable_macro.repair_hint.contains("create_part"));
    assert!(editable_macro.retryable);
    assert_eq!(editable_macro.validate(), Ok(()));
    assert!(app.assistant_proposal.is_none());

    let selection = AssistantValidationSelection {
        mode: "selected",
        requested: BTreeSet::new(),
        unknown: vec!["mystery".to_owned()],
    };
    let validator = app
        .derive_assistant_preview_plan(&AssistantPreviewSource::ValidationRepair(selection))
        .unwrap_err();
    assert_eq!(validator.phase, AssistantRejectionPhase::DomainValidation);
    assert_eq!(validator.code, "validator.selection_invalid");
    assert_eq!(validator.validate(), Ok(()));
}

#[test]
fn structured_rejection_is_localized_and_drives_exactly_one_bounded_replan() {
    struct RejectedReplanTransport {
        contexts: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl AssistantTransport for RejectedReplanTransport {
        fn chat(
            &self,
            _handshake: AssistantHandshake,
            _request_id: &str,
            _message: &str,
            context: &serde_json::Value,
            _cancellation: AssistantCancellation,
        ) -> Result<AssistantChatResult, String> {
            self.contexts.lock().unwrap().push(context.clone());
            Ok(AssistantChatResult {
                message: "Still targets a missing occurrence.".to_owned(),
                model_intent: Some(missing_occurrence_model_intent()),
            })
        }
    }

    fn missing_occurrence_model_intent() -> AssistantModelIntent {
        AssistantModelIntent {
            replace_scene: false,
            boxes: Vec::new(),
            translations: vec![
                ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                    occurrence_id: 999,
                    delta_mm: [10.0, 0.0, 0.0],
                },
            ],
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }
    }

    let transport = Arc::new(RejectedReplanTransport {
        contexts: std::sync::Mutex::new(Vec::new()),
    });
    let mut app = KetchupApp::new();
    app.assistant_transport = transport.clone();
    app.assistant_messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Move occurrence 999.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );
    app.assistant_pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: None,
        result: AssistantChatResult {
            message: "Moved it.".to_owned(),
            model_intent: Some(missing_occurrence_model_intent()),
        },
        message: "Move occurrence 999.".to_owned(),
        replan_attempted: false,
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    let context = egui::Context::default();

    app.poll_assistant_chat(&context);

    let first_diagnostic = app
        .assistant_messages
        .last()
        .and_then(|message| message.diagnostic.clone())
        .expect("the first rejection preserves its structured diagnostic");
    assert_eq!(first_diagnostic.code, "canonical.occurrence_not_found");
    assert!(app.assistant_chat_task.is_some());
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.assistant_pending_execution.is_none()
        && app.assistant_chat_task.is_some()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
        app.poll_assistant_chat(&context);
    }
    assert!(app.assistant_pending_execution.is_some());

    let contexts = transport.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0]["assistant_replan"]["attempt"], 1);
    assert_eq!(contexts[0]["assistant_replan"]["max_attempts"], 1);
    assert_eq!(
        contexts[0]["assistant_replan"]["diagnostic"],
        serde_json::to_value(&first_diagnostic).unwrap()
    );
    assert!(assistant_context_byte_length(&contexts[0]) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES);
    drop(contexts);

    app.poll_assistant_chat(&context);

    assert!(app.assistant_chat_task.is_none());
    assert!(app.assistant_pending_execution.is_none());
    assert_eq!(transport.contexts.lock().unwrap().len(), 1);
    assert_eq!(
        app.assistant_messages
            .iter()
            .filter(|message| message.diagnostic.is_some())
            .count(),
        2
    );
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );
    let english = app.localized_assistant_rejection(&first_diagnostic, true);
    let slovak = KetchupApp::with_catalog(LocaleCatalog::slovak())
        .localized_assistant_rejection(&first_diagnostic, true);
    for text in [english, slovak] {
        assert!(text.contains("canonical.occurrence_not_found"));
        assert!(text.contains("occurrence 999 does not exist"));
    }
}

#[test]
fn canonical_error_codes_are_stable_machine_identifiers() {
    assert_eq!(
        CanonicalError::OccurrenceInAssemblyMate(OccurrenceId(17)).code(),
        "canonical.occurrence_in_assembly_mate"
    );
    assert_eq!(
        CanonicalError::DefinitionNotFound(DefinitionId(9)).code(),
        "canonical.definition_not_found"
    );
}

#[test]
fn assistant_path_resolution_skips_a_stale_higher_priority_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let stale = directory.path().join("stale.exe");
    let installed = directory.path().join("KetchupPrivateAssistant.exe");
    std::fs::write(&installed, b"sidecar").unwrap();

    assert_eq!(
        first_existing_assistant_path([Some(stale), Some(installed.clone()), None]),
        Some(installed)
    );
}

#[test]
fn export_rollback_preserves_concurrent_destination_and_original_backup() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("model.step");
    std::fs::write(&target, b"original").unwrap();
    let original_sha256 = export_target_sha256(&target).unwrap();
    let mut backup = export_backup(&target, original_sha256.as_deref()).unwrap();
    let backup_path = backup.as_ref().unwrap().to_path_buf();
    std::fs::write(&target, b"concurrent writer").unwrap();

    let error = restore_export_backup(&target, &mut backup, None).unwrap_err();

    assert!(error.contains("changed concurrently"));
    assert_eq!(std::fs::read(&target).unwrap(), b"concurrent writer");
    assert_eq!(std::fs::read(backup_path).unwrap(), b"original");
    assert!(backup.is_none());
}

#[test]
fn export_rollback_replaces_only_its_own_published_artifact() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("model.step");
    std::fs::write(&target, b"original").unwrap();
    let original_sha256 = export_target_sha256(&target).unwrap();
    let mut backup = export_backup(&target, original_sha256.as_deref()).unwrap();
    let published = b"ketchup export";
    std::fs::write(&target, published).unwrap();
    let published_sha256 = ketchup_core::graph::sha256_hex(published);

    restore_export_backup(&target, &mut backup, Some(&published_sha256)).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(backup.is_none());
}

#[test]
fn interaction_projection_refresh_defers_while_the_current_frame_reads_the_cache() {
    let app = KetchupApp::new();
    let snapshot = app.document.current();
    app.refresh_interaction_projection_cache(&snapshot);
    let expected_revision = snapshot.revision_id();
    let expected_exact_stamp = app.exact_results.contents_stamp();
    {
        let mut cache = app.interaction_projection_cache.borrow_mut();
        let cache = cache.as_mut().expect("the initial projection is cached");
        cache.revision_id = expected_revision.wrapping_add(1);
        cache.exact_results_stamp = expected_exact_stamp.wrapping_add(1);
    }

    let current_frame = app.interaction_projection_cache.borrow();
    app.refresh_interaction_projection_cache(&snapshot);
    let deferred = current_frame
        .as_ref()
        .expect("an active reader keeps the last valid projection");
    assert_ne!(deferred.revision_id, expected_revision);
    assert_ne!(deferred.exact_results_stamp, expected_exact_stamp);
    drop(current_frame);

    app.refresh_interaction_projection_cache(&snapshot);
    let refreshed = app.interaction_projection_cache.borrow();
    let refreshed = refreshed
        .as_ref()
        .expect("the deferred projection rebuild completes next frame");
    assert_eq!(refreshed.revision_id, expected_revision);
    assert_eq!(refreshed.exact_results_stamp, expected_exact_stamp);
}

#[test]
fn only_body_producing_features_are_export_candidates() {
    assert!(!FeatureKind::Profile { points_mm: vec![] }.produces_body());
    assert!(
        !FeatureKind::BottleProfileControl {
            profile: FeatureId(1),
            body_radius: Dimension::new("1", 1.0).unwrap(),
            body_height: Dimension::new("1", 1.0).unwrap(),
            shoulder_rise: Dimension::new("1", 1.0).unwrap(),
        }
        .produces_body()
    );
    assert!(
        FeatureKind::Extrusion {
            profile: FeatureId(1),
            height: Dimension::new("1", 1.0).unwrap(),
        }
        .produces_body()
    );
}

fn apply_reviewed_model_intent(app: &mut KetchupApp, intent: AssistantModelIntent) -> bool {
    app.prepare_assistant_model_intent(intent) && app.confirm_assistant_proposal()
}

fn select_initial_top_face(app: &mut KetchupApp) {
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
}

fn install_graph_result(
    app: &mut KetchupApp,
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
    fallback_bounds_mm: Option<[[f64; 3]; 2]>,
) {
    let snapshot = app.document.current();
    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition_id, producer_feature_id).unwrap();
    let [minimum_mm, maximum_mm] = graph
        .producer_bounds_mm()
        .unwrap()
        .or(fallback_bounds_mm)
        .unwrap();
    let minimum = Vec3::new(minimum_mm[0], minimum_mm[1], minimum_mm[2]);
    let maximum = Vec3::new(maximum_mm[0], maximum_mm[1], maximum_mm[2]);
    let size = maximum - minimum;
    let vertices_mm = box_corners(size.x, size.y, size.z)
        .map(|point| {
            let point = point + minimum;
            [point.x, point.y, point.z]
        })
        .to_vec();
    let triangles = [
        ([0, 2, 1], 0),
        ([1, 2, 3], 0),
        ([4, 5, 6], 1),
        ([5, 7, 6], 1),
        ([0, 1, 4], 2),
        ([1, 5, 4], 2),
        ([2, 6, 3], 3),
        ([3, 6, 7], 3),
        ([0, 4, 2], 4),
        ([2, 4, 6], 4),
        ([1, 3, 5], 5),
        ([3, 7, 5], 5),
    ]
    .into_iter()
    .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
        vertex_indices,
        face_ordinal,
    })
    .collect();
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            exact_input_digest: "headless-topology-input".into(),
            result_fingerprint: "headless-topology-result".into(),
            volume_mm3: size.x * size.y * size.z,
            topology_counts: [8, 12, 6, 1, 1],
            bounds_mm: [
                [minimum.x, minimum.y, minimum.z],
                [maximum.x, maximum.y, maximum.z],
            ],
            backend: "headless-topology-backend.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
        &StepImportMesh {
            vertices_mm,
            triangles,
        },
    )
    .unwrap();
    assert!(app.headless_install_exact_package(ExactBodyPackage::Graph(package)));
}

fn install_initial_graph_result(app: &mut KetchupApp) {
    install_graph_result(app, INITIAL_BOX_DEFINITION, FeatureId(2), None);
}

fn select_initial_topological(app: &mut KetchupApp, kind: TopologicalElementKind, ordinal: u32) {
    assert!(app.select_topological_locator(TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind,
        ordinal,
    }));
}

#[test]
fn manual_and_assistant_push_pull_share_the_identical_canonical_batch() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.set_push_pull_distance_input("15");
    assert!(app.start_preview());
    let manual = app.smart_push_pull_proposal.as_ref().unwrap().clone();
    app.cancel_preview();

    assert!(
        app.prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
            target: FeatureId(2),
            value_text: "35".to_owned(),
        })
    );
    let assistant = app.assistant_proposal.as_ref().unwrap();
    assert_eq!(manual.batch(), assistant.batch());
    assert_eq!(manual.command_digest(), assistant.command_digest());
    assert_eq!(manual.principal(), ProposalPrincipal::ManualClient);
    assert_eq!(assistant.principal(), ProposalPrincipal::LocalAssistant);
    assert_eq!(app.document_revision(), manual.provenance_revision());
}

#[test]
fn assistant_preview_plan_rejects_source_proposal_stale_and_replay_atomically() {
    fn state(app: &KetchupApp) -> (u64, String, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.document.visible_undo_steps(),
        )
    }

    let visibility_intent = WorkflowIntent::SetOccurrenceVisibility {
        target: OccurrenceId(1),
        visible: false,
    };

    let mut proposal_tampered = KetchupApp::new();
    assert!(proposal_tampered.prepare_assistant_intent(visibility_intent.clone()));
    let mut visibility_plan = proposal_tampered.assistant_proposal.take().unwrap();
    assert!(
        proposal_tampered.prepare_assistant_intent(WorkflowIntent::RenameDefinition {
            target: INITIAL_BOX_DEFINITION,
            name: "Malicious replacement".to_owned(),
        })
    );
    visibility_plan.proposal = proposal_tampered
        .assistant_proposal
        .take()
        .unwrap()
        .proposal;
    let before_tamper = state(&proposal_tampered);
    proposal_tampered.assistant_proposal = Some(visibility_plan);
    assert!(!proposal_tampered.confirm_assistant_proposal());
    assert_eq!(state(&proposal_tampered), before_tamper);

    let mut source_tampered = KetchupApp::new();
    assert!(source_tampered.prepare_assistant_intent(visibility_intent.clone()));
    let mut plan = source_tampered.assistant_proposal.take().unwrap();
    plan.source = AssistantPreviewSource::Workflow(WorkflowIntent::SetOccurrenceVisibility {
        target: OccurrenceId(1),
        visible: true,
    });
    let before_source_tamper = state(&source_tampered);
    source_tampered.assistant_proposal = Some(plan);
    assert!(!source_tampered.confirm_assistant_proposal());
    assert_eq!(state(&source_tampered), before_source_tamper);

    let mut model_tampered = KetchupApp::new();
    let model_intent = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: vec![
            ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                occurrence_id: 1,
                delta_mm: [5.0, 0.0, 0.0],
            },
        ],
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    };
    assert!(model_tampered.prepare_assistant_model_intent(model_intent));
    let mut model_plan = model_tampered.assistant_proposal.take().unwrap();
    assert!(model_tampered.prepare_assistant_intent(visibility_intent.clone()));
    model_plan.proposal = model_tampered.assistant_proposal.take().unwrap().proposal;
    let before_model_tamper = state(&model_tampered);
    model_tampered.assistant_proposal = Some(model_plan);
    assert!(!model_tampered.confirm_assistant_proposal());
    assert_eq!(state(&model_tampered), before_model_tamper);

    let mut stale = KetchupApp::new();
    assert!(stale.prepare_assistant_intent(visibility_intent.clone()));
    assert!(stale.create_box());
    let before_stale = state(&stale);
    assert!(!stale.confirm_assistant_proposal());
    assert_eq!(state(&stale), before_stale);

    let mut valid = KetchupApp::new();
    let initial = state(&valid);
    assert!(valid.prepare_assistant_intent(visibility_intent));
    let replay = valid.assistant_proposal.clone().unwrap();
    assert!(valid.confirm_assistant_proposal());
    assert_eq!(valid.document.visible_undo_steps(), initial.2 + 1);
    assert!(
        !valid
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .visible()
    );
    let committed = state(&valid);
    valid.assistant_proposal = Some(replay);
    assert!(!valid.confirm_assistant_proposal());
    assert_eq!(state(&valid), committed);
    assert!(valid.undo());
    assert_eq!(state(&valid), initial);
}

#[test]
fn assistant_conversation_changes_participate_in_document_dirty_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("chat-dirty.ketchup");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));

    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());
    app.assistant_messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Create a shelf.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    assert!(app.is_dirty());
    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());
    app.new_assistant_chat();
    assert!(app.is_dirty());
}

#[test]
fn assistant_context_exposes_bounded_identified_read_only_agent_state_view() {
    let app = KetchupApp::new();
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );

    let context = app.assistant_context();
    let state_view = context["state_view"].as_object().unwrap();
    let content = state_view["content"].as_str().unwrap();
    assert_eq!(state_view["format"], AGENT_STATE_VIEW_V1);
    assert_eq!(state_view["complete"], true);
    assert_eq!(state_view["byte_length"], content.len());
    assert_eq!(
        state_view["sha256"],
        ketchup_core::graph::sha256_hex(content.as_bytes())
    );
    assert!(content.starts_with("schema=ketchup.state-view.agent.v1\n"));
    assert!(content.contains(&format!(
        "source.canonical_digest={}",
        app.document.current().canonical_digest()
    )));
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );

    let oversized = "semantic.line=bounded\n".repeat(MAX_ASSISTANT_STATE_VIEW_BYTES);
    let bounded = bounded_assistant_state_view(&oversized);
    let bounded_content = bounded["content"].as_str().unwrap();
    assert_eq!(bounded["complete"], false);
    assert_eq!(bounded["byte_length"], oversized.len());
    assert_eq!(
        bounded["sha256"],
        ketchup_core::graph::sha256_hex(oversized.as_bytes())
    );
    assert!(bounded_content.len() <= MAX_ASSISTANT_STATE_VIEW_BYTES);
    assert!(bounded_content.ends_with('\n'));
}

#[test]
fn assistant_context_exposes_the_selected_group_for_generic_rotation() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "Rotatable group".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(GroupId(1)),
            },
        ]))
        .unwrap();
    assert!(app.select_group(GroupId(1)));

    let context = app.assistant_context();
    assert_eq!(context["selected_group_id"], 1);
}

#[test]
fn provider_context_remains_bounded_for_extreme_selection_and_history() {
    let long_text = "x".repeat(4 * 1024);
    let occurrences = (1..=100)
        .map(|id| serde_json::json!({ "occurrence_id": id, "name": long_text }))
        .collect::<Vec<_>>();
    let conversation = (0..20)
        .map(|_| serde_json::json!({ "role": "assistant", "text": long_text }))
        .collect::<Vec<_>>();
    let issues = (0..100)
        .map(|id| serde_json::json!({ "code": "test", "evidence": long_text, "id": id }))
        .collect::<Vec<_>>();
    let context = serde_json::json!({
        "document_id": 1,
        "revision": 1,
        "canonical_digest": "digest",
        "state_view": {
            "format": AGENT_STATE_VIEW_V1,
            "complete": false,
            "byte_length": long_text.len(),
            "sha256": "digest",
            "content": long_text,
        },
        "project_memory": {
            "schema": ASSISTANT_MEMORY_SCHEMA,
            "document_id": 1,
            "stored_count": 0,
            "retrieved_count": 0,
            "complete": true,
            "byte_length": 2,
            "entries": [],
        },
        "validation": {
            "schema": "ketchup.assistant-validation-context.v1",
            "complete": true,
            "issues_complete": true,
            "issue_count": issues.len(),
            "issues": issues,
        },
        "selected_occurrence_ids": (1..=100).collect::<Vec<_>>(),
        "selected_group_id": 1,
        "selected_profile_translation_target": null,
        "selected_parameter_edit_target": null,
        "occurrence_count": 100,
        "occurrences_complete": true,
        "occurrences": occurrences,
        "boxes": [{ "name": long_text }],
        "conversation": conversation,
    });

    let bounded = bounded_assistant_provider_context(context);
    assert!(assistant_context_byte_length(&bounded) <= MAX_ASSISTANT_PROVIDER_CONTEXT_BYTES);
    assert_eq!(bounded["context_complete"], false);
    assert_eq!(bounded["occurrences_complete"], false);
    assert_eq!(bounded["validation"]["complete"], false);
    assert_eq!(bounded["validation"]["details_truncated"], true);
}

#[test]
fn only_successful_assistant_completion_enters_project_memory() {
    let mut app = KetchupApp::new();
    app.assistant_messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Remember the shelf spacing.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            result: AssistantChatResult {
                message: "The shelf spacing is 320 mm.".to_owned(),
                model_intent: None,
            },
            diagnostics: None,
        }))
        .unwrap();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    app.poll_assistant_chat(&egui::Context::default());
    assert_eq!(app.assistant_memory.entries.len(), 1);
    assert!(app.container_data.extensions().any(|entry| {
        entry.namespace() == ASSISTANT_CHAT_NAMESPACE && entry.path() == ASSISTANT_MEMORY_PATH
    }));

    let mut failed = KetchupApp::new();
    failed.assistant_messages.push(AssistantChatMessage {
        role: AssistantMessageRole::User,
        text: "Do not remember a failed request.".to_owned(),
        source: "test".to_owned(),
        diagnostic: None,
    });
    let (sender, receiver) = mpsc::channel();
    sender.send(Err("provider unavailable".to_owned())).unwrap();
    failed.assistant_chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: failed.document.current().document_id(),
        revision_id: failed.document.current().revision_id(),
        canonical_digest: failed.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    failed.poll_assistant_chat(&egui::Context::default());
    assert!(failed.assistant_memory.entries.is_empty());
}

#[test]
fn assistant_project_memory_retrieval_is_bounded_relevant_and_read_only() {
    let mut app = KetchupApp::new();
    for index in 0..140 {
        app.assistant_memory.remember(
            &format!("Fixture note {index}"),
            &format!("Fixture answer {index}"),
        );
    }
    app.assistant_memory
        .remember("Remember shelf spacing", "The shelf spacing is 320 mm.");
    let before = (
        app.document.current().revision_id(),
        app.document.current().canonical_digest(),
        app.document.visible_undo_steps(),
    );

    let context = app.assistant_context_for("What is the shelf spacing?");
    let memory = context["project_memory"].as_object().unwrap();
    let entries = memory["entries"].as_array().unwrap();
    assert_eq!(memory["schema"], ASSISTANT_MEMORY_SCHEMA);
    assert_eq!(
        memory["document_id"],
        app.document.current().document_id().0
    );
    assert_eq!(memory["stored_count"], MAX_ASSISTANT_MEMORY_ENTRIES);
    assert!(entries.len() <= MAX_ASSISTANT_MEMORY_RETRIEVAL_ENTRIES);
    assert!(entries.iter().any(|entry| {
        entry["assistant"] == "The shelf spacing is 320 mm."
            && entry["sha256"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64)
    }));
    let encoded = serde_json::to_vec(entries).unwrap();
    assert_eq!(memory["byte_length"], encoded.len());
    assert!(encoded.len() <= MAX_ASSISTANT_MEMORY_RETRIEVAL_BYTES);
    assert_eq!(
        (
            app.document.current().revision_id(),
            app.document.current().canonical_digest(),
            app.document.visible_undo_steps(),
        ),
        before
    );
}

#[test]
fn assistant_project_memory_persists_in_its_document_and_rejects_foreign_scope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-memory.ketchup");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    app.assistant_memory
        .remember("Shelf material", "Use birch plywood.");
    assert!(app.save_document_to(&path));

    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_from(&path));
    let context = reopened.assistant_context_for("Which shelf material?");
    let memory = context["project_memory"]["entries"].as_array().unwrap();
    assert_eq!(memory.len(), 1);
    assert_eq!(memory[0]["assistant"], "Use birch plywood.");
    reopened.new_assistant_chat();
    assert_eq!(
        reopened.assistant_context_for("material")["project_memory"]["retrieved_count"],
        1
    );

    let document_id = reopened.document.current().document_id().0;
    let foreign = AssistantProjectMemory::empty(document_id + 1);
    let entry = ketchup_core::persistence::ExtensionEntry::new(
        ASSISTANT_CHAT_NAMESPACE,
        ASSISTANT_MEMORY_PATH,
        false,
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    reopened.container_data.set_extension(entry);
    let before = (
        reopened.document.current().revision_id(),
        reopened.document.current().canonical_digest(),
        reopened.document.visible_undo_steps(),
    );
    reopened.load_assistant_memory();
    assert_eq!(reopened.assistant_memory.document_id, document_id);
    assert!(reopened.assistant_memory.entries.is_empty());
    assert_eq!(
        (
            reopened.document.current().revision_id(),
            reopened.document.current().canonical_digest(),
            reopened.document.visible_undo_steps(),
        ),
        before
    );
}

#[test]
fn new_chat_cancels_the_active_assistant_request() {
    let mut app = KetchupApp::new();
    let cancellation = AssistantCancellation::default();
    let (_sender, receiver) = mpsc::channel();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.new_assistant_chat();

    assert!(cancellation.is_cancelled());
    assert!(app.assistant_chat_task.is_none());
}

#[test]
fn new_and_open_cancel_active_assistant_requests() {
    let mut app = KetchupApp::new();
    let new_cancellation = AssistantCancellation::default();
    let (_new_sender, new_receiver) = mpsc::channel();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver: new_receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: new_cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.new_document();

    assert!(new_cancellation.is_cancelled());
    assert!(app.assistant_chat_task.is_none());

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("assistant-open-cancel.ketchup");
    assert!(app.save_document_to(&path));
    let open_cancellation = AssistantCancellation::default();
    let (_open_sender, open_receiver) = mpsc::channel();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver: open_receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: open_cancellation.clone(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    assert!(app.open_document_from(&path));

    assert!(open_cancellation.is_cancelled());
    assert!(app.assistant_chat_task.is_none());
}

#[test]
fn assistant_progress_phases_are_accessible_with_deterministic_channels() {
    let mut app = KetchupApp::new();
    let requesting = app.catalog.text("assistant-progress-requesting");
    let elapsed = app.catalog.format(
        "assistant-progress-elapsed",
        &BTreeMap::from([("time", "0:07".to_owned())]),
    );
    let executing = app.catalog.text("assistant-progress-executing");
    assert_ne!(
        assistant_clock_frame(Duration::ZERO),
        assistant_clock_frame(Duration::from_millis(250))
    );
    let (_sender, receiver) = mpsc::channel();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now() - Duration::from_secs(7),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: app.document.current().revision_id(),
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);

    harness.step();

    assert!(
        harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(&requesting)
                        || node.value().as_deref() == Some(&requesting))
            })
            .next()
            .is_some()
    );
    assert!(
        harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(&elapsed)
                        || node.value().as_deref() == Some(&elapsed))
            })
            .next()
            .is_some()
    );
    let state = harness.state_mut();
    state.assistant_chat_task = None;
    state.assistant_pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: None,
        message: "test".to_owned(),
        replan_attempted: false,
        result: AssistantChatResult {
            message: "Moved it.".to_owned(),
            model_intent: Some(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: vec![
                    ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                        occurrence_id: 1,
                        delta_mm: [25.0, 0.0, 0.0],
                    },
                ],
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
                balloon_texts: Vec::new(),
            }),
        },
        document_id: state.document.current().document_id(),
        revision_id: state.document.current().revision_id(),
        canonical_digest: state.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    // The executing phase lasts exactly one frame: the panel announces it,
    // and `poll_assistant_chat` consumes the pending execution at the end of
    // that same frame. `run` would keep stepping while anything still asks
    // for an immediate repaint, so it must not be used to observe a state
    // that is gone by the next frame.
    harness.step();

    assert!(
        harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(&executing)
                        || node.value().as_deref() == Some(&executing))
            })
            .next()
            .is_some()
    );
    assert!(
        harness.state().assistant_pending_execution.is_none(),
        "the announced execution must be consumed by the frame that announced it"
    );
}

#[test]
fn new_chat_discards_a_pending_assistant_execution_before_commit() {
    let mut app = KetchupApp::new();
    let revision = app.document.current().revision_id();
    app.assistant_pending_execution = Some(AssistantPendingExecution {
        cad_edit_program: None,
        message: "test".to_owned(),
        replan_attempted: false,
        result: AssistantChatResult {
            message: "Moved it.".to_owned(),
            model_intent: Some(AssistantModelIntent {
                replace_scene: false,
                boxes: Vec::new(),
                translations: vec![
                    ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                        occurrence_id: 1,
                        delta_mm: [25.0, 0.0, 0.0],
                    },
                ],
                rotations: Vec::new(),
                profile_translations: Vec::new(),
                parameter_edits: Vec::new(),
                linear_arrays: Vec::new(),
                bottles: Vec::new(),
                gable_roofs: Vec::new(),
                staircases: Vec::new(),
                oriented_beams: Vec::new(),
                balloon_texts: Vec::new(),
            }),
        },
        document_id: app.document.current().document_id(),
        revision_id: revision,
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });

    app.new_assistant_chat();
    app.poll_assistant_chat(&egui::Context::default());

    assert!(app.assistant_pending_execution.is_none());
    assert_eq!(app.document.current().revision_id(), revision);
    assert!(app.assistant_verification.is_none());
}

#[test]
fn assistant_model_change_requires_explicit_confirmation_after_validation() {
    let mut app = KetchupApp::new();
    let revision = app.document.current().revision_id();
    let (sender, receiver) = mpsc::channel();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation: AssistantCancellation::default(),
        document_id: app.document.current().document_id(),
        revision_id: revision,
        canonical_digest: app.document.current().canonical_digest(),
        source: "test".to_owned(),
    });
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            result: AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![
                        ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                            occurrence_id: 1,
                            delta_mm: [25.0, 0.0, 0.0],
                        },
                    ],
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                    bottles: Vec::new(),
                    gable_roofs: Vec::new(),
                    staircases: Vec::new(),
                    oriented_beams: Vec::new(),
                    balloon_texts: Vec::new(),
                }),
            },
            diagnostics: None,
        }))
        .unwrap();
    let context = egui::Context::default();

    app.poll_assistant_chat(&context);

    assert!(app.assistant_chat_task.is_none());
    assert!(app.assistant_pending_execution.is_some());
    assert_eq!(app.document.current().revision_id(), revision);

    app.poll_assistant_chat(&context);

    assert!(app.assistant_pending_execution.is_none());
    assert_eq!(app.document.current().revision_id(), revision);
    assert!(app.assistant_proposal.is_some());
    assert!(app.assistant_verification.is_none());
    assert!(app.assistant_messages.iter().any(|message| {
        message.role == AssistantMessageRole::Assistant && message.text == "Moved it."
    }));

    assert!(app.confirm_assistant_proposal());
    assert_eq!(app.document.current().revision_id(), revision + 1);
    assert!(app.assistant_verification.is_some());
}

#[test]
fn assistant_replace_scene_clears_collection_references_in_the_same_undo_step() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(1);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: collection,
                name: "Original selection".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: collection,
                occurrence_ids: vec![OccurrenceId(1)],
            },
        ]))
        .unwrap();
    let before = app.document.current();

    assert!(apply_reviewed_model_intent(
        &mut app,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![AssistantBoxIntent {
                name: "Replacement".to_owned(),
                size_mm: [100.0, 80.0, 60.0],
                origin_mm: [0.0, 0.0, 0.0],
                subtract_boxes: Vec::new(),
            }],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }
    ));

    let replaced = app.document.current();
    assert_eq!(replaced.revision_id(), before.revision_id() + 1);
    assert_eq!(replaced.occurrences().count(), 1);
    assert_eq!(replaced.definitions().count(), 1);
    assert_eq!(
        replaced
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OccurrenceId(1)]
    );
    assert_eq!(
        app.document.current().canonical_digest(),
        before.canonical_digest()
    );
}

#[test]
fn stale_assistant_model_result_is_reported_without_mutating_the_newer_document() {
    let mut app = KetchupApp::new();
    let request_document_id = app.document.current().document_id();
    let request_revision_id = app.document.current().revision_id();
    let request_digest = app.document.current().canonical_digest();
    let (sender, receiver) = mpsc::channel();
    let cancellation = AssistantCancellation::default();
    app.assistant_chat_task = Some(AssistantChatTask {
        receiver,
        selected_occurrence_ids: Vec::new(),
        request_id: "test".to_owned(),
        message: "test".to_owned(),
        replan_attempted: false,
        started_at: Instant::now(),
        cancellation,
        document_id: request_document_id,
        revision_id: request_revision_id,
        canonical_digest: request_digest,
        source: "test".to_owned(),
    });
    assert!(
        app.prepare_assistant_intent(WorkflowIntent::SetOccurrenceTranslation {
            target: OccurrenceId(1),
            x_mm_text: "10".to_owned(),
            y_mm_text: "0".to_owned(),
            z_mm_text: "0".to_owned(),
        })
    );
    assert!(app.confirm_assistant_proposal());
    let changed_revision = app.document.current().revision_id();
    let changed_digest = app.document.current().canonical_digest();
    let undo_steps = app.document.visible_undo_steps();
    sender
        .send(Ok(AssistantTransportResponse {
            cad_edit_program: None,
            result: AssistantChatResult {
                message: "Moved it.".to_owned(),
                model_intent: Some(AssistantModelIntent {
                    replace_scene: false,
                    boxes: Vec::new(),
                    translations: vec![
                        ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                            occurrence_id: 1,
                            delta_mm: [100.0, 0.0, 0.0],
                        },
                    ],
                    rotations: Vec::new(),
                    profile_translations: Vec::new(),
                    parameter_edits: Vec::new(),
                    linear_arrays: Vec::new(),
                    bottles: Vec::new(),
                    gable_roofs: Vec::new(),
                    staircases: Vec::new(),
                    oriented_beams: Vec::new(),
                    balloon_texts: Vec::new(),
                }),
            },
            diagnostics: None,
        }))
        .unwrap();

    let context = egui::Context::default();
    app.poll_assistant_chat(&context);
    app.poll_assistant_chat(&context);

    assert_eq!(app.document.current().revision_id(), changed_revision);
    assert_eq!(app.document.current().canonical_digest(), changed_digest);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);
    assert!(app.assistant_messages.iter().any(|message| {
        message.role == AssistantMessageRole::Error
            && message.text == app.catalog.text("assistant-error-stale-response")
    }));
}

#[test]
fn assistant_conversation_round_trips_with_its_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("chat-model.ketchup");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    app.assistant_messages = vec![
        AssistantChatMessage {
            role: AssistantMessageRole::User,
            text: "Posuň hranol o 100 mm.".to_owned(),
            source: "Codex OAuth · gpt-test".to_owned(),
            diagnostic: None,
        },
        AssistantChatMessage {
            role: AssistantMessageRole::Error,
            text: "Výskyt 999 neexistuje.".to_owned(),
            source: "Codex OAuth · gpt-test".to_owned(),
            diagnostic: Some(AssistantRejectionDiagnostic {
                phase: AssistantRejectionPhase::CanonicalValidation,
                code: "canonical.occurrence_not_found".to_owned(),
                operation: "translate_occurrence".to_owned(),
                target: "occurrence:999".to_owned(),
                failed_invariant: "occurrence 999 does not exist".to_owned(),
                repair_hint: "Choose an occurrence that exists.".to_owned(),
                retryable: true,
            }),
        },
    ];

    assert!(app.save_document_to(&path));
    let mut reopened = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    assert!(reopened.open_document_from(&path));
    assert_eq!(reopened.assistant_messages, app.assistant_messages);
    assert_eq!(reopened.document_path.as_deref(), Some(path.as_path()));

    reopened.new_assistant_chat();
    assert!(reopened.assistant_messages.is_empty());
    assert!(reopened.save_document_to(&path));
    let mut cleared = KetchupApp::new();
    assert!(cleared.open_document_from(&path));
    assert!(cleared.assistant_messages.is_empty());
}

// Palette contrast is proved once for all four appearances in
// `theme::tests::every_palette_keeps_text_and_accent_legible`, so this file
// no longer keeps a second copy of the thresholds for one hardcoded set.

#[test]
fn switching_theme_repaints_the_shell_without_touching_the_document() {
    let mut app = KetchupApp::new();
    let before_revision = app.document_revision();
    let before_digest = app.canonical_digest();
    let graphite = app.palette();

    for kind in ThemeKind::ALL {
        app.set_theme(kind);
        assert_eq!(app.theme(), kind);
        assert_eq!(app.palette(), Palette::of(kind));
        assert_eq!(
            app.document_revision(),
            before_revision,
            "changing appearance must not commit a canonical batch"
        );
        assert_eq!(
            app.canonical_digest(),
            before_digest,
            "changing appearance must not change the model"
        );
    }

    app.set_theme(ThemeKind::Graphite);
    assert_eq!(app.palette(), graphite);
    assert!(
        !app.can_undo(),
        "appearance must never enter the undo stack"
    );
}

fn lossy_legacy_document() -> Vec<u8> {
    let mut bytes = b"KETCHUPDOC".to_vec();
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&7_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&42_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(b'x');
    bytes.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}

#[cfg(feature = "named-product-fixtures")]
fn current_bottle_package(app: &KetchupApp, definition_id: DefinitionId) -> Arc<ExactBodyPackage> {
    use ketchup_core::exact_product::canonical_reference_lineage_digest;
    use ketchup_core::exact_revolve::{SHELL_FACE_ROLES, build_revolve_package};

    let snapshot = app.document.current();
    let request = ExactRevolveRequest::from_snapshot(&snapshot, definition_id).unwrap();
    let points = request.points_mm();
    let max_radius = points.iter().map(|point| point[0]).fold(0.0_f64, f64::max);
    let evidence = SHELL_FACE_ROLES
        .map(|role| {
            (
                role,
                canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                ),
                format!("geometry-{role:?}"),
            )
        })
        .to_vec();
    Arc::new(
        build_revolve_package(
            &request,
            "exact-input".to_owned(),
            "result".to_owned(),
            "OCCT-test".to_owned(),
            "linear=1e-7mm".to_owned(),
            [
                [-max_radius, -max_radius, points[0][1]],
                [max_radius, max_radius, points[5][1]],
            ],
            evidence,
        )
        .unwrap()
        .into(),
    )
}

fn through_cut_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(10),
                name: "Exact cut body".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(10),
                name: "Outer profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(12),
                definition_id: DefinitionId(10),
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(11),
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(13),
                definition_id: DefinitionId(10),
                name: "Cut profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(14),
                definition_id: DefinitionId(10),
                name: "Through cut".to_owned(),
                kind: FeatureKind::ThroughCut {
                    target: FeatureId(12),
                    profile: FeatureId(13),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(10),
                definition_id: DefinitionId(10),
                name: "Cut body occurrence".to_owned(),
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

fn exact_worker_executable() -> PathBuf {
    let executable_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(executable_name)
}

fn current_box_package(app: &KetchupApp) -> Arc<ketchup_core::exact_product::ExactRenderPackage> {
    use ketchup_core::exact_product::{
        build_box_render_package, canonical_reference_lineage_digest,
    };

    let snapshot = app.document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION)
        .expect("the default box has an exact request");
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                request.document_id,
                request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                "planar_face",
            ),
            format!("geometry-{role:?}"),
        )
    });
    Arc::new(
        build_box_render_package(
            &request,
            "exact-input".to_owned(),
            "result".to_owned(),
            "backend".to_owned(),
            "tolerance".to_owned(),
            [[0.0; 3], request.dimensions_mm()],
            evidence,
        )
        .expect("the exact package matches the default box"),
    )
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn exact_bottle_can_start_preview_and_commit_the_standard_move_tool() {
    let mut app = KetchupApp::new();
    assert!(app.create_bottle());
    let definition_id = app.selected_bottle_definition().unwrap();
    let bottle_path = app.selection.occurrences.iter().next().unwrap().clone();
    let package = current_bottle_package(&app, definition_id);
    let snapshot = app.document.current();
    app.exact_results
        .insert_current(&snapshot, package)
        .unwrap();
    assert!(
        app.active_boxes()
            .iter()
            .all(|item| item.instance_path != bottle_path),
        "the exact bottle intentionally has no canonical box proxy"
    );

    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1_000.0, 800.0));
    let pointer = app.project(Vec3::new(120.0, 0.0, 50.0), rect);
    app.update_viewport_inference(Some(pointer), rect);
    assert_eq!(
        app.hovered
            .as_ref()
            .map(|selection| &selection.instance_path),
        Some(&bottle_path)
    );
    assert!(app.begin_move_drag_at(pointer, rect, false));
    let mut drag = app.move_drag.take().unwrap();
    drag.delta_mm = Vec3::new(25.0, 10.0, 0.0);
    let overrides = app.document.current().scene_query();
    app.move_drag = Some(drag.clone());
    let preview = app.move_preview_transform_overrides();
    let original = overrides
        .iter()
        .find(|occurrence| occurrence.instance_path == bottle_path)
        .unwrap()
        .transform;
    assert_eq!(
        preview[&bottle_path].matrix()[3],
        original.matrix()[3] + 25.0
    );
    assert_eq!(
        preview[&bottle_path].matrix()[7],
        original.matrix()[7] + 10.0
    );
    let render_snapshot = app.document.current();
    let render_plan = InstancedRenderPlan::from_snapshot_with_transform_overrides(
        &render_snapshot,
        &app.exact_results,
        &mut app.render_cache,
        &preview,
    );
    let bottle_instance = &render_plan
        .batches()
        .iter()
        .find(|batch| batch.definition_id == definition_id)
        .unwrap()
        .instances[0];
    assert_eq!(
        bottle_instance.transform[3],
        (original.matrix()[3] + 25.0) as f32
    );
    assert_eq!(
        bottle_instance.transform[7],
        (original.matrix()[7] + 10.0) as f32
    );

    app.move_drag = None;
    assert!(app.commit_move_drag(&drag));
    let moved = app
        .document
        .current()
        .world_transform_for_occurrence(bottle_path.root_occurrence())
        .unwrap();
    assert_eq!(moved.matrix()[3], original.matrix()[3] + 25.0);
    assert_eq!(moved.matrix()[7], original.matrix()[7] + 10.0);
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn bottle_numeric_workflow_is_atomic_and_round_trips_losslessly() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("editable-bottle.ketchup");
    let mut app = KetchupApp::new();
    let undo_before_create = app.document.visible_undo_steps();

    assert!(app.create_bottle());
    assert_eq!(app.document.visible_undo_steps(), undo_before_create + 1);
    let definition_id = app.selected_bottle_definition().unwrap();
    let revision_before_edit = app.document.current().revision_id();
    let undo_before_edit = app.document.visible_undo_steps();

    assert!(app.set_bottle_parameters(&BottleEditorInputs {
        definition_id,
        body_radius: "34 mm".to_owned(),
        body_height: "125".to_owned(),
        shoulder_rise: "16.5".to_owned(),
        thickness: "2.5".to_owned(),
        finish_amount: "1.5".to_owned(),
        finish_kind: BottleEdgeFinishKind::Chamfer,
    }));
    assert_ne!(app.document.current().revision_id(), revision_before_edit);
    assert_eq!(app.document.visible_undo_steps(), undo_before_edit + 1);
    let edited = app.document.current();
    let ids = KetchupApp::bottle_feature_ids(&edited, definition_id).unwrap();
    let FeatureKind::BottleProfileControl {
        body_radius,
        body_height,
        shoulder_rise,
        ..
    } = edited.feature(ids.control).unwrap().kind()
    else {
        panic!("bottle control feature missing");
    };
    assert_eq!(body_radius.source_token(), "34 mm");
    assert_eq!(body_height.millimetres(), 125.0);
    assert_eq!(shoulder_rise.millimetres(), 16.5);
    assert!(matches!(
        edited.feature(ids.finish).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Chamfer,
            ..
        }
    ));

    let digest_before_rejection = edited.canonical_digest();
    let revision_before_rejection = edited.revision_id();
    let undo_before_rejection = app.document.visible_undo_steps();
    assert!(!app.set_bottle_parameters(&BottleEditorInputs {
        definition_id,
        body_radius: "34".to_owned(),
        body_height: "125".to_owned(),
        shoulder_rise: "16.5".to_owned(),
        thickness: "7".to_owned(),
        finish_amount: "1.5".to_owned(),
        finish_kind: BottleEdgeFinishKind::Fillet,
    }));
    assert_eq!(
        app.document.current().canonical_digest(),
        digest_before_rejection
    );
    assert_eq!(
        app.document.current().revision_id(),
        revision_before_rejection
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before_rejection);

    assert!(app.save_document_to(&path));
    let expected = app.document.current();
    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_from(&path));
    let actual = reopened.document.current();
    assert_eq!(actual.canonical_digest(), expected.canonical_digest());
    assert!(ExactRevolveRequest::from_snapshot(&actual, definition_id).is_ok());
    assert!(KetchupApp::bottle_editor_inputs(&actual, definition_id).is_some());
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn accepted_bottle_result_drives_render_pick_authority_and_fail_closed_exports() {
    let directory = tempfile::tempdir().unwrap();
    let exact_path = directory.path().join("bottle.kbex");
    let mesh_path = directory.path().join("bottle.obj");
    let stale_path = directory.path().join("stale.kbex");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(61),
    ));
    assert!(app.create_bottle());
    let definition_id = app.selected_bottle_definition().unwrap();
    let package = current_bottle_package(&app, definition_id);
    let snapshot = app.document.current();
    app.exact_results
        .insert_current(&snapshot, package)
        .unwrap();

    let report = app.bottle_authority_report(definition_id).unwrap();
    assert!(report.current);
    assert!(report.validation_passed);
    assert_eq!(report.durable_reference_count, 9);
    assert_eq!(app.exact_render_body_count(), 1);
    assert_eq!(app.exact_stable_reference_count(), 9);
    let picked = app
        .exact_pick_durable(
            Ray::new(Vec3::new(130.0, 0.0, 50.0), Vec3::new(-1.0, 0.0, 0.0)).unwrap(),
        )
        .expect("accepted bottle mesh must remain exactly pickable");
    assert_eq!(picked.body.role(), Some(ExactFaceRole::ShellOuterBody));

    assert!(app.export_bottle_exact_recipe_to(definition_id, &exact_path));
    assert!(app.export_bottle_mesh_to(definition_id, &mesh_path));
    let exact = std::fs::read_to_string(&exact_path).unwrap();
    let mesh = std::fs::read_to_string(&mesh_path).unwrap();
    let loss = std::fs::read_to_string(mesh_path.with_extension("obj.loss.txt")).unwrap();
    assert!(exact.starts_with("KETCHUP_EXACT_BOTTLE_RECIPE_V1\n"));
    assert!(exact.contains("result_fingerprint=result"));
    assert!(mesh.contains("# authority=accepted exact OCCT B-Rep"));
    assert!(mesh.contains("g shell.outer.body"));
    assert!(loss.contains("editability_loss="));
    assert!(loss.contains("topology_loss="));
    assert!(loss.contains("tolerance_loss="));

    let ids = KetchupApp::bottle_feature_ids(&app.document.current(), definition_id).unwrap();
    let undo_before_drag = app.document.visible_undo_steps();
    assert!(app.commit_bottle_direct_drag(
        BottleDirectDrag {
            definition_id,
            feature_id: ids.control,
            control: BottleControlDimension::BodyRadius,
            pointer_start: Pos2::ZERO,
            value_start_mm: 30.0,
            screen_direction: Vec2::X,
            pixels_per_mm: 1.0,
        },
        33.0,
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before_drag + 1);
    assert_eq!(app.exact_render_body_count(), 0);
    assert!(!app.bottle_authority_report(definition_id).unwrap().current);
    assert!(!app.export_bottle_exact_recipe_to(definition_id, &stale_path));
    assert!(!stale_path.exists());
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn lossy_mesh_export_requires_payload_bound_receipt_before_any_artifact_write() {
    let directory = tempfile::tempdir().unwrap();
    let mesh_path = directory.path().join("protected.obj");
    let loss_path = mesh_path.with_extension("obj.loss.txt");
    let original_mesh = b"preserve mesh until approval".to_vec();
    let original_loss = b"preserve loss report until approval".to_vec();
    std::fs::write(&mesh_path, &original_mesh).unwrap();
    std::fs::write(&loss_path, &original_loss).unwrap();
    let script = dialogs::ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(73);
    let mut app = KetchupApp::new().with_dialogs(Box::new(script.clone()));
    assert!(app.create_bottle());
    let definition_id = app.selected_bottle_definition().unwrap();
    let package = current_bottle_package(&app, definition_id);
    let snapshot = app.document.current();
    app.exact_results
        .insert_current(&snapshot, package)
        .unwrap();
    let canonical_before = app.document.current().canonical_digest();
    let revision_before = app.document.current().revision_id();
    let undo_before = app.document.visible_undo_steps();

    assert!(!app.export_bottle_mesh_to(definition_id, &mesh_path));
    assert_eq!(std::fs::read(&mesh_path).unwrap(), original_mesh);
    assert_eq!(std::fs::read(&loss_path).unwrap(), original_loss);
    assert!(app.last_side_effect_receipt().is_none());

    assert!(app.export_bottle_mesh_to(definition_id, &mesh_path));
    let receipt = app
        .last_side_effect_receipt()
        .expect("approved lossy export returns an authorization receipt");
    assert_eq!(receipt.approving_human(), 73);
    assert_eq!(receipt.revision_id(), revision_before);
    assert_eq!(receipt.operation(), "export-lossy-obj-with-loss-report");
    assert_eq!(receipt.scope().class(), HighRiskClass::LossyConversion);
    assert_eq!(
        receipt.scope().path(),
        Some(mesh_path.display().to_string().as_str())
    );
    assert_ne!(std::fs::read(&mesh_path).unwrap(), original_mesh);
    assert_ne!(std::fs::read(&loss_path).unwrap(), original_loss);
    assert_eq!(app.document.current().canonical_digest(), canonical_before);
    assert_eq!(app.document.current().revision_id(), revision_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);
    assert_eq!(script.high_risk_prompts().len(), 2);
    assert!(script.high_risk_prompts()[0].contains("Payload SHA-256:"));
}

#[test]
fn current_exact_occurrence_suppresses_only_the_non_preview_proxy() {
    let mut app = KetchupApp::new();
    let package = current_box_package(&app);
    let snapshot = app.document.current();
    app.exact_results
        .insert_current(&snapshot, Arc::new((*package).clone().into()))
        .unwrap();
    let exact_projection = app.exact_projection(&snapshot);

    assert!(exact_projection.contains_occurrence(&InstancePath::root(OccurrenceId(1))));
    assert!(app.viewport_boxes(&exact_projection).is_empty());

    app.selection.primary = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    assert_eq!(app.viewport_boxes(&exact_projection).len(), 1);
}

#[test]
fn exact_occurrence_reference_and_mesh_export_use_the_canonical_world_transform() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transformed.obj");
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(62),
    ));
    let transform = Transform::from_matrix([
        0.0, -1.0, 0.0, 10.0, 1.0, 0.0, 0.0, 20.0, 0.0, 0.0, 1.0, 30.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform,
            },
        ]))
        .unwrap();
    let package = current_box_package(&app);
    let snapshot = app.document.current();
    app.exact_results
        .insert_current(&snapshot, Arc::new((*package).clone().into()))
        .unwrap();
    let instance_path = InstancePath::root(OccurrenceId(1));

    let reference = app
        .exact_reference_for_occurrence(&instance_path, ExactFaceRole::Top)
        .unwrap();
    assert_eq!(reference.instance_path, instance_path);
    assert_eq!(reference.body.role(), Some(ExactFaceRole::Top));
    assert!(app.export_exact_occurrence_mesh_to(&instance_path, &path));

    let mesh = std::fs::read_to_string(&path).unwrap();
    let first_vertex = mesh
        .lines()
        .find_map(|line| line.strip_prefix("v "))
        .unwrap()
        .split_whitespace()
        .map(|value| value.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(first_vertex, vec![10.0, 20.0, 30.0]);
    assert!(mesh.contains("g extrusion.top"));
    let loss = std::fs::read_to_string(path.with_extension("obj.loss.txt")).unwrap();
    assert!(loss.contains("exact-body-to-world-space-mesh"));
    assert!(loss.contains("producer_feature_id=2"));
}

#[test]
fn sketchup_scene_import_confirmation_rederives_the_exact_reviewed_plan_atomically() {
    fn pending_for(app: &KetchupApp, path: &Path, source: &[u8]) -> PendingSketchupSceneImport {
        let snapshot = app.document.current();
        let source = SketchupSceneImportSourcePlan {
            path: path.to_owned(),
            source: source.to_vec(),
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            source_sha256: sha256_bytes(source),
            source_byte_len: source.len() as u64,
        };
        PendingSketchupSceneImport {
            plan: app
                .prepare_sketchup_scene_import_preview_plan(source)
                .unwrap(),
            invalidated: false,
        }
    }

    fn state(app: &KetchupApp) -> (u64, String, usize, usize, usize, usize, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.redo_step_count(),
            app.definition_count(),
            app.occurrence_count(),
            app.import_receipt_count(),
        )
    }

    let source = br#"{"schema":"ketchup.sketchup-scene.v1","units":"inch","definitions":[{"id":"component:solid:1","name":"Reviewed solid","vertices":[[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],"triangles":[[0,2,1],[0,1,3],[0,3,2],[1,2,3]]}],"instances":[{"definition":"component:solid:1","name":"Reviewed instance","transform":[1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0,0.0,1.0],"visible":true}],"metadata":{"material_assignments":0,"textures":0,"tags":0,"scenes":0,"unsupported_entities":0}}"#;
    let alternate_source = source
        .windows(b"Reviewed instance".len())
        .position(|window| window == b"Reviewed instance")
        .map(|offset| {
            let mut alternate = source.to_vec();
            alternate.splice(
                offset..offset + b"Reviewed instance".len(),
                b"Tampered instance".iter().copied(),
            );
            alternate
        })
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed-plan.kscene");
    std::fs::write(&path, source).unwrap();

    let mut app = KetchupApp::new();
    let pending = pending_for(&app, &path, source);
    let alternate = pending_for(&app, &path, &alternate_source);
    let baseline = state(&app);

    let mut review_tamper = pending.clone();
    review_tamper.plan.review = alternate.plan.review.clone();
    assert!(!app.import_sketchup_scene_from(&review_tamper));
    assert_eq!(state(&app), baseline);

    let mut proposal_tamper = pending.clone();
    proposal_tamper.plan.proposal = alternate.plan.proposal;
    assert!(!app.import_sketchup_scene_from(&proposal_tamper));
    assert_eq!(state(&app), baseline);

    let mut source_tamper = pending.clone();
    source_tamper.plan.source.source[0] ^= 1;
    assert!(!app.import_sketchup_scene_from(&source_tamper));
    assert_eq!(state(&app), baseline);

    let mut stale = pending.clone();
    stale.plan.source.revision_id += 1;
    assert!(!app.import_sketchup_scene_from(&stale));
    assert_eq!(state(&app), baseline);

    assert!(
        app.import_sketchup_scene_from(&pending),
        "{}",
        app.action_digest()
    );
    assert_eq!(app.document_revision(), baseline.0 + 1);
    assert_eq!(app.undo_step_count(), baseline.2 + 1);
    let committed = state(&app);
    assert!(!app.import_sketchup_scene_from(&pending));
    assert_eq!(state(&app), committed);
}

#[test]
fn exact_step_preview_plan_rejects_tamper_stale_and_replay_atomically() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let source = std::fs::read(&path).unwrap();
    let executable = exact_worker_executable();
    assert!(executable.is_file(), "{}", executable.display());
    let mut app = KetchupApp::new();
    app.connect_exact_worker(executable).unwrap();
    let snapshot = app.document.current();
    let source_plan = StepImportSourcePlan {
        path,
        source_sha256: sha256_bytes(&source),
        source_byte_len: source.len() as u64,
        source,
        document_id: snapshot.document_id(),
        revision_id: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
    };
    let pending = PendingStepImport {
        plan: app.prepare_step_import_preview_plan(source_plan).unwrap(),
        invalidated: false,
    };
    let baseline_revision = app.document_revision();
    let baseline_digest = app.canonical_digest();
    let baseline_undo = app.undo_step_count();
    let baseline_container = app.container_data.clone();
    let assert_unchanged = |app: &KetchupApp| {
        assert_eq!(app.document_revision(), baseline_revision);
        assert_eq!(app.canonical_digest(), baseline_digest);
        assert_eq!(app.undo_step_count(), baseline_undo);
        assert_eq!(app.container_data, baseline_container);
    };

    let mut evidence_tamper = pending.clone();
    evidence_tamper.plan.evidence.solid_count += 1;
    assert!(!app.import_step_from(&evidence_tamper));
    assert_unchanged(&app);

    let mut proposal_tamper = pending.clone();
    proposal_tamper.plan.proposal = app
        .document
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            }]),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    assert!(!app.import_step_from(&proposal_tamper));
    assert_unchanged(&app);

    let mut blob_tamper = pending.clone();
    blob_tamper.plan.blob_hash = "tampered".to_owned();
    assert!(!app.import_step_from(&blob_tamper));
    assert_unchanged(&app);

    let mut source_tamper = pending.clone();
    source_tamper.plan.source.source[0] ^= 1;
    assert!(!app.import_step_from(&source_tamper));
    assert_unchanged(&app);

    let mut stale = pending.clone();
    stale.plan.source.revision_id += 1;
    assert!(!app.import_step_from(&stale));
    assert_unchanged(&app);

    assert!(app.import_step_from(&pending), "{}", app.action_digest());
    assert_eq!(app.document_revision(), baseline_revision + 1);
    assert_eq!(app.undo_step_count(), baseline_undo + 1);
    assert_eq!(
        app.container_data.blobs().get(&pending.plan.blob_hash),
        Some(&pending.plan.source.source)
    );
    let committed_revision = app.document_revision();
    let committed_digest = app.canonical_digest();
    let committed_undo = app.undo_step_count();
    let committed_container = app.container_data.clone();
    assert!(!app.import_step_from(&pending));
    assert_eq!(app.document_revision(), committed_revision);
    assert_eq!(app.canonical_digest(), committed_digest);
    assert_eq!(app.undo_step_count(), committed_undo);
    assert_eq!(app.container_data, committed_container);
}

#[test]
fn stl_import_confirmation_rederives_the_exact_reviewed_plan_atomically() {
    fn pending_for(
        app: &KetchupApp,
        path: &Path,
        source: &[u8],
        unit: ImportLengthUnit,
    ) -> PendingStlImport {
        let snapshot = app.document.current();
        let source = StlImportSourcePlan {
            path: path.to_owned(),
            source: source.to_vec(),
            unit,
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            source_sha256: sha256_bytes(source),
            source_byte_len: source.len() as u64,
        };
        PendingStlImport {
            plan: app.prepare_stl_import_preview_plan(source).unwrap(),
            review_error: None,
            invalidated: false,
        }
    }

    fn state(app: &KetchupApp) -> (u64, String, usize, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.redo_step_count(),
        )
    }

    let source = b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 1\n   vertex 0 1 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 1 0 0\n   vertex 0 1 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
endsolid tetrahedron\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed-plan.stl");
    std::fs::write(&path, source).unwrap();

    let mut app = KetchupApp::new();
    let pending = pending_for(&app, &path, source, ImportLengthUnit::Millimetre);
    let baseline = state(&app);

    let mut unit_tamper = pending.clone();
    unit_tamper.plan.source.unit = ImportLengthUnit::Centimetre;
    assert!(!app.import_stl_from(&unit_tamper));
    assert_eq!(state(&app), baseline);

    let alternate = pending_for(&app, &path, source, ImportLengthUnit::Centimetre);
    let mut review_tamper = pending.clone();
    review_tamper.plan.review = alternate.plan.review.clone();
    assert!(!app.import_stl_from(&review_tamper));
    assert_eq!(state(&app), baseline);

    let mut proposal_tamper = pending.clone();
    proposal_tamper.plan.proposal = alternate.plan.proposal;
    assert!(!app.import_stl_from(&proposal_tamper));
    assert_eq!(state(&app), baseline);

    let mut source_tamper = pending.clone();
    source_tamper.plan.source.source[0] ^= 1;
    assert!(!app.import_stl_from(&source_tamper));
    assert_eq!(state(&app), baseline);

    let mut stale = pending.clone();
    stale.plan.source.revision_id += 1;
    assert!(!app.import_stl_from(&stale));
    assert_eq!(state(&app), baseline);

    assert!(app.import_stl_from(&pending), "{}", app.action_digest());
    assert_eq!(app.document_revision(), baseline.0 + 1);
    assert_eq!(app.undo_step_count(), baseline.2 + 1);
    let committed = state(&app);
    assert!(!app.import_stl_from(&pending));
    assert_eq!(state(&app), committed);
}

#[test]
fn imported_stl_mesh_is_rendered_picked_and_outlined_from_canonical_geometry() {
    let source = b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 1\n   vertex 0 1 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 1 0 0\n   vertex 0 1 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
endsolid tetrahedron\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron.stl");
    std::fs::write(&path, source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let source_plan = StlImportSourcePlan {
        path,
        source: source.to_vec(),
        unit: ImportLengthUnit::Millimetre,
        document_id: snapshot.document_id(),
        revision_id: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        source_sha256: sha256_bytes(source),
        source_byte_len: source.len() as u64,
    };
    let pending = PendingStlImport {
        plan: app.prepare_stl_import_preview_plan(source_plan).unwrap(),
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_stl_from(&pending));

    let snapshot = app.document.current();
    let occurrence = snapshot.occurrences().next().unwrap().id();
    let projection = MeshInteractionProjection::from_snapshot(&snapshot);
    let hit = projection
        .exact_surface_pick(Ray::new(Vec3::new(0.2, 0.2, 2.0), Vec3::new(0.0, 0.0, -1.0)).unwrap())
        .expect("the imported tetrahedron must be pickable by its canonical triangles");
    assert_eq!(hit.instance_path, InstancePath::root(occurrence));

    let mut render_cache = renderer::DerivedRenderCache::default();
    let render_plan = renderer::InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact_results,
        &mut render_cache,
    );
    let imported_batch = render_plan
        .batches()
        .iter()
        .find(|batch| batch.definition_id == hit.definition_id)
        .expect("the imported mesh must produce a render batch");
    assert_eq!(imported_batch.geometry.index_count(), 12);
    assert!(imported_batch.geometry.vertex_count() >= 4);
    assert_eq!(imported_batch.instances.len(), 1);

    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.zoom_fit();
    app.selection.select_occurrence(occurrence, false);
    assert!(selection_stroke_segments(&context, &mut app) > 0);
}

/// A closed jagged sphere: every edge separates differently oriented
/// triangles, so the whole tessellation counts as feature edges.
fn jagged_sphere_binary_stl(stacks: usize, slices: usize) -> Vec<u8> {
    let point = |stack: usize, slice: usize| {
        if stack == 0 {
            return [0.0, 0.0, 10.0];
        }
        if stack == stacks {
            return [0.0, 0.0, -10.0];
        }
        let radius = 10.0 + f64::from(u8::from((stack + slice).is_multiple_of(2))) * 1.5;
        let polar = std::f64::consts::PI * stack as f64 / stacks as f64;
        let azimuth = std::f64::consts::TAU * (slice % slices) as f64 / slices as f64;
        [
            radius * polar.sin() * azimuth.cos(),
            radius * polar.sin() * azimuth.sin(),
            radius * polar.cos(),
        ]
    };
    let mut facets = Vec::new();
    for stack in 0..stacks {
        for slice in 0..slices {
            let corners = [
                point(stack, slice),
                point(stack, slice + 1),
                point(stack + 1, slice + 1),
                point(stack + 1, slice),
            ];
            if stack > 0 {
                facets.push([corners[0], corners[2], corners[1]]);
            }
            if stack + 1 < stacks {
                facets.push([corners[0], corners[3], corners[2]]);
            }
        }
    }
    let mut source = vec![0_u8; 80];
    source.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for facet in &facets {
        let normal = triangle_normal(facet.map(|point| Vec3::new(point[0], point[1], point[2])));
        for value in [normal.x, normal.y, normal.z] {
            source.extend_from_slice(&(value as f32).to_le_bytes());
        }
        for corner in facet {
            for value in corner {
                source.extend_from_slice(&(*value as f32).to_le_bytes());
            }
        }
        source.extend_from_slice(&0_u16.to_le_bytes());
    }
    source
}

/// Hovering or selecting a body must not re-derive its feature edges every
/// frame: on an imported mesh that cost seconds per frame and froze the
/// viewport as soon as the pointer touched the model.
#[test]
fn selected_imported_mesh_paints_its_outline_without_per_frame_edge_derivation() {
    let source = jagged_sphere_binary_stl(90, 90);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("jagged-sphere.stl");
    std::fs::write(&path, &source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let source_plan = StlImportSourcePlan {
        path,
        source: source.clone(),
        unit: ImportLengthUnit::Millimetre,
        document_id: snapshot.document_id(),
        revision_id: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        source_sha256: sha256_bytes(&source),
        source_byte_len: source.len() as u64,
    };
    let pending = PendingStlImport {
        plan: app.prepare_stl_import_preview_plan(source_plan).unwrap(),
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_stl_from(&pending), "{}", app.action_digest());

    let snapshot = app.document.current();
    let mesh = definition_mesh_body(&snapshot, snapshot.definitions().next().unwrap().id())
        .expect("the import produces a canonical mesh body");
    assert!(mesh.triangles.len() > 10_000);
    let occurrence = snapshot.occurrences().next().unwrap().id();
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.zoom_fit();
    app.selection.select_occurrence(occurrence, false);
    assert!(selection_stroke_segments(&context, &mut app) > 0);

    let cached = Arc::clone(
        &app.overlay_edge_cache
            .borrow()
            .values()
            .next()
            .unwrap()
            .edges,
    );
    let started = Instant::now();
    for _ in 0..8 {
        let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    }
    let elapsed = started.elapsed();
    assert!(
        Arc::ptr_eq(
            &cached,
            &app.overlay_edge_cache
                .borrow()
                .values()
                .next()
                .unwrap()
                .edges
        ),
        "the feature edges of an unchanged body must be derived once, not per frame"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "eight frames of a selected {} triangle mesh took {elapsed:?}",
        mesh.triangles.len()
    );
}

#[test]
fn dxf_import_confirmation_rederives_the_exact_reviewed_plan_atomically() {
    fn pending_for(
        app: &KetchupApp,
        path: &Path,
        source: &[u8],
        unit: ImportLengthUnit,
    ) -> PendingDxfImport {
        let snapshot = app.document.current();
        let source = DxfImportSourcePlan {
            path: path.to_owned(),
            source: source.to_vec(),
            unit,
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            source_sha256: sha256_bytes(source),
            source_byte_len: source.len() as u64,
        };
        PendingDxfImport {
            plan: app.prepare_dxf_import_preview_plan(source).unwrap(),
            unit_confirmed: true,
            review_error: None,
            invalidated: false,
        }
    }

    fn state(app: &KetchupApp) -> (u64, String, usize, usize) {
        (
            app.document_revision(),
            app.canonical_digest(),
            app.undo_step_count(),
            app.redo_step_count(),
        )
    }

    let source = b"0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nreviewed\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nENDSEC\n0\nEOF\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed-plan.dxf");
    std::fs::write(&path, source).unwrap();

    let mut valid = KetchupApp::new();
    let pending = pending_for(&valid, &path, source, ImportLengthUnit::Millimetre);
    let initial = state(&valid);
    assert!(valid.import_dxf_from(&pending));
    assert_eq!(valid.document_revision(), initial.0 + 1);
    assert_eq!(valid.undo_step_count(), initial.2 + 1);
    let committed = state(&valid);
    assert!(!valid.import_dxf_from(&pending));
    assert_eq!(state(&valid), committed);

    let mut unit_tampered = KetchupApp::new();
    let mut pending = pending_for(&unit_tampered, &path, source, ImportLengthUnit::Millimetre);
    pending.plan.source.unit = ImportLengthUnit::Centimetre;
    let before = state(&unit_tampered);
    assert!(!unit_tampered.import_dxf_from(&pending));
    assert_eq!(state(&unit_tampered), before);

    let mut review_tampered = KetchupApp::new();
    let mut pending = pending_for(
        &review_tampered,
        &path,
        source,
        ImportLengthUnit::Millimetre,
    );
    let alternate = pending_for(
        &review_tampered,
        &path,
        source,
        ImportLengthUnit::Centimetre,
    );
    pending.plan.review = alternate.plan.review;
    let before = state(&review_tampered);
    assert!(!review_tampered.import_dxf_from(&pending));
    assert_eq!(state(&review_tampered), before);

    let mut proposal_tampered = KetchupApp::new();
    let mut pending = pending_for(
        &proposal_tampered,
        &path,
        source,
        ImportLengthUnit::Millimetre,
    );
    let alternate = pending_for(
        &proposal_tampered,
        &path,
        source,
        ImportLengthUnit::Centimetre,
    );
    pending.plan.proposal = alternate.plan.proposal;
    let before = state(&proposal_tampered);
    assert!(!proposal_tampered.import_dxf_from(&pending));
    assert_eq!(state(&proposal_tampered), before);

    let mut unconfirmed = KetchupApp::new();
    let mut pending = pending_for(&unconfirmed, &path, source, ImportLengthUnit::Millimetre);
    pending.unit_confirmed = false;
    let before = state(&unconfirmed);
    assert!(!unconfirmed.import_dxf_from(&pending));
    assert_eq!(state(&unconfirmed), before);

    let mut stale = KetchupApp::new();
    let pending = pending_for(&stale, &path, source, ImportLengthUnit::Millimetre);
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let after_drift = state(&stale);
    assert!(!stale.import_dxf_from(&pending));
    assert_eq!(state(&stale), after_drift);
}

#[test]
fn imported_dxf_profiles_remain_projected_and_pickable_after_persistence() {
    let source = b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nopen\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nLWPOLYLINE\n8\nclosed\n90\n3\n70\n1\n\
10\n20\n20\n0\n10\n30\n20\n0\n10\n20\n20\n10\n\
0\nCIRCLE\n8\nround\n10\n50\n20\n5\n40\n4\n\
0\nENDSEC\n0\nEOF\n";
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    std::fs::write(&path, source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let source_plan = DxfImportSourcePlan {
        path,
        source: source.to_vec(),
        unit: ImportLengthUnit::Millimetre,
        document_id: snapshot.document_id(),
        revision_id: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        source_sha256: sha256_bytes(source),
        source_byte_len: source.len() as u64,
    };
    let pending = PendingDxfImport {
        plan: app.prepare_dxf_import_preview_plan(source_plan).unwrap(),
        unit_confirmed: true,
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_dxf_from(&pending));

    let committed = app.document.current();
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&committed)).unwrap();
    let snapshot = reopened.snapshot();
    let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
    let scene = projection.scene().unwrap();
    assert_eq!(scene.occurrence_count(), 3);
    assert!(snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        exact_circle_geometry(segments, *closed) == Some(([50.0, 5.0], 4.0))
    }));
    let open_hit = scene
        .exact_pick(
            Ray::new(Vec3::new(5.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .expect("the persisted open DXF profile must remain pickable");
    let closed_hit = scene
        .exact_pick(
            Ray::new(Vec3::new(25.0, 5.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .expect("the persisted closed DXF profile must remain pickable");
    let circle_hit = scene
        .exact_pick(
            Ray::new(Vec3::new(54.0, 5.0, 10.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            0.01,
        )
        .expect("the persisted exact DXF circle must remain pickable");
    assert_ne!(
        open_hit.primary.reference.instance_path,
        closed_hit.primary.reference.instance_path
    );
    assert_ne!(
        closed_hit.primary.reference.instance_path,
        circle_hit.primary.reference.instance_path
    );
}

#[test]
fn hovering_and_selecting_a_canonical_mesh_body_paints_its_outline() {
    let source = jagged_sphere_binary_stl(6, 8);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("small-sphere.stl");
    std::fs::write(&path, &source).unwrap();
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let snapshot = app.document.current();
    let pending = PendingStlImport {
        plan: app
            .prepare_stl_import_preview_plan(StlImportSourcePlan {
                path,
                source: source.clone(),
                unit: ImportLengthUnit::Millimetre,
                document_id: snapshot.document_id(),
                revision_id: snapshot.revision_id(),
                canonical_digest: snapshot.canonical_digest(),
                source_sha256: sha256_bytes(&source),
                source_byte_len: source.len() as u64,
            })
            .unwrap(),
        review_error: None,
        invalidated: false,
    };
    assert!(app.import_stl_from(&pending), "{}", app.action_digest());
    let snapshot = app.document.current();
    let occurrence = snapshot.occurrences().next().unwrap().id();
    assert!(
        definition_mesh_body(&snapshot, snapshot.definitions().next().unwrap().id()).is_some(),
        "the imported sphere is stored as a canonical mesh body"
    );
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.zoom_fit();

    let unselected = selection_stroke_segments(&context, &mut app);
    app.selection.select_occurrence(occurrence, false);
    let selected = selection_stroke_segments(&context, &mut app);

    assert_eq!(unselected, 0);
    assert!(
        selected > 0,
        "a selected canonical mesh body must paint its selection outline"
    );
}

/// The Rotate tool is useless if the user cannot see what will turn and
/// about which axis, so the protractor is asserted on the painted frame
/// rather than trusted because the state exists.
#[test]
fn the_armed_rotate_tool_paints_a_protractor_on_the_axis_it_will_turn_about() {
    let mut app = KetchupApp::new();
    let context = egui::Context::default();
    select_initial_top_face(&mut app);

    let blue = guide_tick_colour(Axis::Z);
    let red = guide_tick_colour(Axis::X);
    assert_eq!(
        painted_segments(&context, &mut app, blue),
        0,
        "no other tool may paint a rotation protractor"
    );

    app.dispatch_command(AppCommand::Rotate);
    let ticks = (360.0 / ROTATION_SNAP_DEGREES).round() as usize;
    assert_eq!(
        painted_segments(&context, &mut app, blue),
        ticks,
        "arming Rotate on a selection must show the blue Z protractor before any gesture"
    );

    // Pinning an axis has to move the protractor with it, otherwise the
    // arrow keys change the outcome invisibly.
    app.set_rotate_axis_lock(Some(Axis::X));
    assert_eq!(painted_segments(&context, &mut app, blue), 0);
    assert_eq!(painted_segments(&context, &mut app, red), ticks);

    // A gesture in flight must additionally show where it started and how
    // far it has come, which is the part that answers "which way".
    app.set_rotate_axis_lock(Some(Axis::Z));
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1_000.0, 800.0));
    app.viewport_rect = Some(rect);
    let grab = app.project(Vec3::new(90.0, 30.0, 20.0), rect);
    app.update_viewport_inference(Some(grab), rect);
    assert!(app.begin_rotate_drag_at(grab, rect, false));
    let mut drag = app.rotate_drag.take().expect("the gesture has started");
    app.advance_rotation(
        &mut drag,
        app.project(Vec3::new(50.0, 90.0, 20.0), rect),
        rect,
        false,
    );
    app.rotate_drag = Some(drag);
    let guide = app.rotation_guide().expect("a live gesture has a guide");
    assert!(
        guide.start_degrees.is_some(),
        "the starting arm must be drawn so the turn has a visible origin"
    );
    assert!(
        rotation_is_meaningful(guide.angle_degrees),
        "the swept angle must reach the guide: {:?}",
        guide.angle_degrees
    );

    app.dispatch_command(AppCommand::Select);
    app.clear_selection();
    app.dispatch_command(AppCommand::Rotate);
    assert_eq!(
        painted_segments(&context, &mut app, blue),
        0,
        "with nothing selected there is no body to turn and nothing to draw"
    );
}

/// The faint ring-and-tick colour the guide uses, which no other viewport
/// layer paints — the solid axis lines of the ground plane are opaque.
fn guide_tick_colour(axis: Axis) -> Color32 {
    let solid = axis_color(axis);
    Color32::from_rgba_unmultiplied(solid.r(), solid.g(), solid.b(), 130)
}

fn painted_segments(context: &egui::Context, app: &mut KetchupApp, colour: Color32) -> usize {
    let output = context.run(egui::RawInput::default(), |context| app.ui(context));
    let mut count = 0;
    for clipped in output.shapes {
        count_segments_coloured(&clipped.shape, colour, &mut count);
    }
    count
}

fn count_segments_coloured(shape: &egui::Shape, colour: Color32, count: &mut usize) {
    match shape {
        egui::Shape::LineSegment { stroke, .. } if stroke.color == colour => *count += 1,
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                count_segments_coloured(shape, colour, count);
            }
        }
        _ => {}
    }
}

fn selection_stroke_segments(context: &egui::Context, app: &mut KetchupApp) -> usize {
    let output = context.run(egui::RawInput::default(), |context| app.ui(context));
    let mut count = 0;
    for clipped in output.shapes {
        count_selection_segments(&clipped.shape, &mut count);
    }
    count
}

fn count_selection_segments(shape: &egui::Shape, count: &mut usize) {
    match shape {
        egui::Shape::LineSegment { stroke, .. } => {
            if stroke.color == Color32::from_rgb(240, 78, 35) {
                *count += 1;
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                count_selection_segments(shape, count);
            }
        }
        _ => {}
    }
}

#[test]
fn production_exact_refresh_uses_graph_for_a_general_boolean_chain() {
    let executable = exact_worker_executable();
    assert!(
        executable.is_file(),
        "build workspace all-targets so the exact worker exists at {}",
        executable.display()
    );
    let definition_id = DefinitionId(77);
    let producer_feature_id = FeatureId(30);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: "General boolean".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(10),
                definition_id,
                name: "Base pentagon".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [-12.0, -8.0],
                        [18.0, -6.0],
                        [24.0, 9.0],
                        [3.0, 20.0],
                        [-17.0, 7.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id,
                name: "Unequal base".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(10),
                    height: Dimension::from_decimal("13").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(20),
                definition_id,
                name: "Slanted tool".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-3.0, -15.0], [27.0, 4.0], [5.0, 24.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(21),
                definition_id,
                name: "Unequal tool".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(20),
                    height: Dimension::from_decimal("19").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: producer_feature_id,
                definition_id,
                name: "Generic intersection".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Intersect,
                    target: FeatureId(11),
                    tool: FeatureId(21),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(77),
                definition_id,
                name: "General boolean occurrence".into(),
                transform: Transform::default(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    assert!(
        ExactFeatureChainRequest::from_snapshot_for_producer(
            &document.current(),
            definition_id,
            producer_feature_id,
        )
        .is_err(),
        "the legacy rectangle evaluator must not be able to authorize this chain"
    );

    let mut app = KetchupApp::new();
    app.document = document;
    app.reset_document_presentation();
    app.connect_exact_worker(&executable).unwrap();
    let context = egui::Context::default();
    for _ in 0..200 {
        app.refresh_exact_products(&context);
        if app.exact_results.len() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let snapshot = app.document.current();
    let package = app
        .exact_results
        .get_render(&snapshot, definition_id)
        .expect("the general boolean chain must produce one exact body");
    assert!(matches!(
        package.as_ref(),
        ExactBodyPackage::Graph(package)
            if package.identity.producer_feature_id == producer_feature_id
    ));
    assert!(!package.topological_references().is_empty());
    assert!(
        package
            .mesh_export(Transform::identity())
            .mesh_obj
            .contains("g topological.face.")
    );
}

#[test]
fn running_app_uses_one_exact_cut_body_for_render_pick_and_export() {
    let executable = exact_worker_executable();
    assert!(
        executable.is_file(),
        "build workspace all-targets so the exact worker exists at {}",
        executable.display()
    );
    let mut app = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(63),
    ));
    app.document = through_cut_document();
    app.document
        .configure_human_confirmation_policy(app.confirmation_surface.verifying_key(), 1)
        .unwrap();
    app.reset_document_presentation();
    app.connect_exact_worker(&executable).unwrap();
    let before = app.document.current();
    let context = egui::Context::default();

    for _ in 0..200 {
        app.refresh_exact_products(&context);
        if app.exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.exact_render_body_count(), 1);
    assert_eq!(app.document.current().revision_id(), before.revision_id());
    assert_eq!(
        app.document.current().canonical_digest(),
        before.canonical_digest()
    );
    let projection = app.exact_projection(&app.document.current());
    assert!(
        projection
            .exact_surface_pick(
                Ray::new(Vec3::new(5.0, 5.0, 20.0), Vec3::new(0.0, 0.0, -1.0)).unwrap()
            )
            .is_none(),
        "the exact through-hole must not be filled by an axis-aligned proxy"
    );
    let wall = app
        .exact_pick_durable(Ray::new(Vec3::new(5.0, 5.0, 5.0), Vec3::new(1.0, 0.0, 0.0)).unwrap())
        .expect("the cut wall must remain durably pickable");
    assert_eq!(wall.body.role(), Some(ExactFaceRole::CutEast));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("through-cut.obj");
    assert!(app.export_exact_occurrence_mesh_to(&InstancePath::root(OccurrenceId(10)), &path));
    let mesh = std::fs::read_to_string(&path).unwrap();
    assert!(mesh.contains("g through_cut.wall.east"));
    assert_eq!(
        mesh.lines().filter(|line| line.starts_with("f ")).count(),
        32
    );
    let loss = std::fs::read_to_string(path.with_extension("obj.loss.txt")).unwrap();
    assert!(loss.contains("authority=accepted exact OCCT B-Rep"));

    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(12),
                dimension: Dimension::from_decimal("11").unwrap(),
            },
        ]))
        .unwrap();
    assert_eq!(app.exact_render_body_count(), 0);
    let stale_projection = app.exact_projection(&app.document.current());
    assert!(!stale_projection.contains_occurrence(&InstancePath::root(OccurrenceId(10))));
    assert!(app.viewport_boxes(&stale_projection).is_empty());
    let stale_path = directory.path().join("stale-through-cut.obj");
    assert!(
        !app.export_exact_occurrence_mesh_to(&InstancePath::root(OccurrenceId(10)), &stale_path)
    );
    assert!(!stale_path.exists());
}

#[test]
fn orbit_passes_both_poles_without_a_pitch_limit() {
    let mut app = KetchupApp::new();

    app.orbit(Vec2::new(0.0, 400.0));
    assert!(app.pitch > 1.2);

    app.orbit(Vec2::new(0.0, -800.0));
    assert!(app.pitch < -1.2);
}

#[test]
fn creating_a_second_box_has_stable_identity_and_undo_redo_visibility() {
    let mut app = KetchupApp::new();
    assert_eq!(app.active_box_count(), 1);

    assert!(app.create_box());
    assert_eq!(app.active_box_count(), 2);
    let created = app.selected_reference().unwrap();
    assert_eq!(created.definition_id, DefinitionId(2));
    assert_eq!(created.instance_path, InstancePath::root(OccurrenceId(2)));
    assert_eq!(app.box_height_mm(created.definition_id), Some(20.0));

    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let second_top = app.project(Vec3::new(125.0, 85.0, 20.0), rect);
    let picked = app.exact_pick_at_screen(second_top, rect).unwrap();
    assert_eq!(picked.definition_id, DefinitionId(2));
    assert_eq!(picked.instance_path, InstancePath::root(OccurrenceId(2)));

    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert_eq!(app.selected_reference(), None);

    assert!(app.redo());
    assert_eq!(app.active_box_count(), 2);
}

#[test]
fn move_rotate_delete_are_independent_undoable_scene_operations() {
    let mut app = KetchupApp::new();
    let selected = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.primary = Some(selected);

    assert!(!app.move_selected(Vec3::ZERO));
    assert!(app.move_selected(Vec3::new(10.0, -5.0, 3.0)));
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(10.0, -5.0, 3.0));
    assert!(app.undo());
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::ZERO);
    assert!(app.redo());
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(10.0, -5.0, 3.0));

    assert!(app.rotate_selected_90());
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));
    assert_eq!(app.active_boxes()[0].origin_mm, Vec3::new(30.0, -25.0, 3.0));
    assert!(app.undo());
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(100.0, 60.0, 20.0));
    assert!(app.redo());
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));

    assert!(app.delete_selected());
    assert_eq!(app.active_box_count(), 0);
    assert_eq!(app.selected_reference(), None);
    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert_eq!(app.selected_reference(), None);
    assert_eq!(app.active_boxes()[0].size_mm, Vec3::new(60.0, 100.0, 20.0));
    assert!(app.redo());
    assert_eq!(app.active_box_count(), 0);
    assert_eq!(app.selected_reference(), None);
}

#[test]
fn push_pull_drag_is_signed_along_the_face_normal() {
    let drag = PushPullDrag {
        source_document_id: DocumentId(1),
        source_revision: 0,
        source_digest: String::new(),
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        pointer_start: Pos2::new(100.0, 100.0),
        extent_start_mm: 20.0,
        screen_normal: Vec2::new(1.0, 0.0),
        pixels_per_mm: 2.0,
    };

    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(120.0, 100.0), true),
        10.0
    );
    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(80.0, 100.0), true),
        -10.0
    );
    assert_eq!(
        push_pull_distance_from_pointer(&drag, Pos2::new(115.0, 100.0), false),
        7.5
    );
}

#[test]
fn push_pull_distance_accepts_units_and_moves_inward() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    app.set_push_pull_distance_input("-5 mm");

    assert!(app.start_preview());
    assert_eq!(
        app.preview_box.as_ref().unwrap().plan.preview_box.size_mm.z,
        15.0
    );
    assert!(app.confirm_preview());
    assert_eq!(app.document_height_mm(), 15.0);
    assert!(app.undo());
    assert_eq!(app.document_height_mm(), 20.0);
}

#[test]
fn push_pull_minimum_side_keeps_the_opposite_face_fixed() {
    let mut app = KetchupApp::new();
    app.selection.primary = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::X,
            side: Side::Minimum,
        },
    });
    let old_maximum = app.active_boxes()[0].origin_mm.x + app.active_boxes()[0].size_mm.x;

    app.set_push_pull_distance_input("30");
    assert!(app.start_preview());
    let preview = app.preview_box.as_ref().unwrap().plan.preview_box.clone();
    assert_eq!(preview.origin_mm.x, -30.0);
    assert_eq!(preview.size_mm.x, 130.0);
    assert_eq!(preview.origin_mm.x + preview.size_mm.x, old_maximum);

    assert!(app.confirm_preview());
    assert_eq!(app.active_boxes()[0], preview);
    assert!(app.undo());
    assert_eq!(
        app.active_boxes()[0].origin_mm.x + app.active_boxes()[0].size_mm.x,
        old_maximum
    );
    assert_eq!(app.active_boxes()[0].size_mm.x, 100.0);
}

#[test]
fn assistant_evaluator_rename_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let node = NodeId(20);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: node,
                name: "width".to_owned(),
                dimension: Dimension::from_decimal("600").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::EvaluatorName;
    app.assistant_target_input = node.0.to_string();
    app.assistant_value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "cabinet width".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::RenameEvaluatorNode(node));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("width".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("cabinet width".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document.current().evaluator_node(node).unwrap().name(),
        "cabinet width"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().evaluator_node(node).unwrap().name(),
        "width"
    );
}

#[test]
fn assistant_evaluator_expression_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let input = NodeId(20);
    let expression = NodeId(21);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: input,
                name: "width".to_owned(),
                dimension: Dimension::from_decimal("600").unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateExpressionNode {
                id: expression,
                name: "double width".to_owned(),
                expression: "$20 * 2".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::EvaluatorExpression;
    app.assistant_target_input = expression.0.to_string();
    app.assistant_value_input = "(".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "$20 * 3".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetEvaluatorExpression(expression)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Text("$20 * 2".to_owned())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("$20 * 3".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .evaluator_node(expression)
            .unwrap()
            .kind()
            .source(),
        "$20 * 3"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .evaluator_node(expression)
            .unwrap()
            .kind()
            .source(),
        "$20 * 2"
    );
}

#[test]
fn assistant_tag_visibility_review_is_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(7);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: tag,
            name: "Hardware".to_owned(),
            visible: true,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::TagVisibility;
    app.assistant_target_input = tag.0.to_string();
    app.assistant_value_input = "yes".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    assert!(
        app.prepare_assistant_intent(WorkflowIntent::SetTagVisibility {
            target: tag,
            visible: false,
        })
    );
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetTagVisibility(tag));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Boolean(true)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Boolean(false)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(!app.document.current().tag(tag).unwrap().visible());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().tag(tag).unwrap().visible());
}

#[test]
fn assistant_occurrence_tag_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(8);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Fixtures".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(tag),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::OccurrenceTag;
    app.assistant_target_input = "1".to_owned();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "none".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceTag(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Tag(Some(tag))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Tag(None)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        None
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        Some(tag)
    );
}

#[test]
fn assistant_occurrence_repoint_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(9);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Alternate".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::OccurrenceDefinition;
    app.assistant_target_input = "1".to_owned();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = definition.0.to_string();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::RepointOccurrence(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Definition(INITIAL_BOX_DEFINITION)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Definition(definition)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .definition_id(),
        definition
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .definition_id(),
        INITIAL_BOX_DEFINITION
    );
}

#[test]
fn assistant_occurrence_parent_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(10);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(group),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::OccurrenceParent;
    app.assistant_target_input = "1".to_owned();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "none".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetOccurrenceParent(OccurrenceId(1))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Group(Some(group))
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Group(None)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .parent(),
        None
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .parent(),
        Some(group)
    );
}

#[test]
fn assistant_group_parent_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(10);
    let parent = GroupId(11);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group,
                name: "Assembly".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: parent,
                name: "Parent".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::GroupParent;
    app.assistant_target_input = group.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = parent.0.to_string();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetGroupParent(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Group(None)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Group(Some(parent))
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document.current().group(group).unwrap().parent(),
        Some(parent)
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().group(group).unwrap().parent(), None);
}

#[test]
fn assistant_group_translation_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(10);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: group,
            name: "Assembly".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::GroupTranslation;
    app.assistant_target_input = group.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "4.5, -2, 11.25".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    let expected = Transform::from_translation(4.5, -2.0, 11.25).unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetGroupTranslation(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Transform(Transform::identity())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Transform(expected)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document.current().group(group).unwrap().transform(),
        expected
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().group(group).unwrap().transform(),
        Transform::identity()
    );
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn assistant_bottle_control_dimension_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    assert!(app.create_bottle());
    let definition_id = app.selected_bottle_definition().unwrap();
    let control = KetchupApp::bottle_feature_ids(&app.document.current(), definition_id)
        .unwrap()
        .control;
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::BottleControlDimension;
    app.assistant_target_input = control.0.to_string();
    app.assistant_value_input = "waist=32".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "body_radius=32".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetBottleControlDimension(control, BottleControlDimension::BodyRadius)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(Dimension::from_decimal("30").unwrap())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(Dimension::from_decimal("32").unwrap())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().feature(control).unwrap().kind(),
        FeatureKind::BottleProfileControl { body_radius, .. }
            if body_radius.millimetres() == 32.0
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(control).unwrap().kind(),
        FeatureKind::BottleProfileControl { body_radius, .. }
            if body_radius.millimetres() == 30.0
    ));
}

#[test]
#[cfg(feature = "named-product-fixtures")]
fn assistant_bottle_finish_kind_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    assert!(app.create_bottle());
    let definition_id = app.selected_bottle_definition().unwrap();
    let finish = KetchupApp::bottle_feature_ids(&app.document.current(), definition_id)
        .unwrap()
        .finish;
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::BottleEdgeFinishKind;
    app.assistant_target_input = finish.0.to_string();
    app.assistant_value_input = "round".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "chamfer".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetBottleEdgeFinishKind(finish)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Fillet)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::BottleEdgeFinishKind(BottleEdgeFinishKind::Chamfer)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().feature(finish).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Chamfer,
            ..
        }
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(finish).unwrap().kind(),
        FeatureKind::BottleEdgeFinish {
            kind: BottleEdgeFinishKind::Fillet,
            ..
        }
    ));
}

#[test]
fn assistant_profile_points_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(50);
    let profile = FeatureId(51);
    let original = vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Assistant profile".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: original.clone(),
                },
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::ProfilePoints;
    app.assistant_target_input = profile.0.to_string();
    app.assistant_value_input = "0,0; invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    let requested = vec![[0.0, 0.0], [12.0, 0.0], [12.0, 8.0], [0.0, 8.0]];
    app.assistant_value_input = "0,0; 12,0; 12,8; 0,8".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetProfilePoints(profile));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ProfilePoints(original.clone())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ProfilePoints(requested.clone())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().feature(profile).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &requested
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(profile).unwrap().kind(),
        FeatureKind::Profile { points_mm } if points_mm == &original
    ));
}

#[test]
fn assistant_rule_outputs_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(30);
    let output = |key: &str| {
        RuleOutput::new(SlotSegment::new(rule, "result", key).unwrap(), Vec::new()).unwrap()
    };
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "layout".to_owned(),
            expression: "1".to_owned(),
            input_ports: vec![PortSpec::number("source").unwrap()],
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: vec![output("left")],
            override_parameters: Vec::new(),
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::RuleOutputs;
    app.assistant_target_input = rule.0.to_string();
    app.assistant_value_input = "result".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);

    let requested = vec![output("center"), output("right")];
    app.assistant_value_input = "result:center; result:right".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::SetRuleOutputs(rule));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::RuleOutputs(vec![output("left")])
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::RuleOutputs(requested.clone())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().evaluator_node(rule).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &requested
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().evaluator_node(rule).unwrap().kind(),
        EvaluatorNodeKind::Rule { outputs, .. } if outputs == &vec![output("left")]
    ));
}

#[test]
fn assistant_create_tag_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateTag;
    app.assistant_target_input = tag.0.to_string();
    app.assistant_value_input = "visible:Reviewed".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "true:Reviewed tag".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateTag(tag));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::TagState {
            name: "Reviewed tag".to_owned(),
            visible: true,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.tag(tag).unwrap();
    assert_eq!(created.name(), "Reviewed tag");
    assert!(created.visible());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().tag(tag).is_none());
}

#[test]
fn assistant_create_collection_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateCollection;
    app.assistant_target_input = collection.0.to_string();
    app.assistant_value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "Reviewed selection".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateCollection(collection));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Reviewed selection".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .name(),
        "Reviewed selection"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().collection(collection).is_none());
}

#[test]
fn assistant_delete_collection_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(24);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: collection,
                name: "Reviewed selection".to_owned(),
            },
            CanonicalCommand::SetCollectionOccurrences {
                id: collection,
                occurrence_ids: vec![OccurrenceId(1)],
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteCollection;
    app.assistant_target_input = collection.0.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteCollection(collection));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::CollectionState {
            name: "Reviewed selection".to_owned(),
            occurrence_ids: vec![OccurrenceId(1)],
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().collection(collection).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.collection(collection).unwrap();
    assert_eq!(restored.name(), "Reviewed selection");
    assert_eq!(
        restored.occurrence_ids().collect::<Vec<_>>(),
        vec![OccurrenceId(1)]
    );
}

#[test]
fn assistant_delete_tag_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let tag = TagId(24);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: tag,
            name: "Reviewed tag".to_owned(),
            visible: false,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteTag;
    app.assistant_target_input = tag.0.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteTag(tag));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::TagState {
            name: "Reviewed tag".to_owned(),
            visible: false,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().tag(tag).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.tag(tag).unwrap();
    assert_eq!(restored.name(), "Reviewed tag");
    assert!(!restored.visible());
}

#[test]
fn assistant_delete_group_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(24);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: group,
            name: "Reviewed group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteGroup;
    app.assistant_target_input = group.0.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteGroup(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::GroupState {
            name: "Reviewed group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().group(group).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.group(group).unwrap();
    assert_eq!(restored.name(), "Reviewed group");
    assert_eq!(restored.transform(), Transform::identity());
    assert_eq!(restored.parent(), None);
}

#[test]
fn assistant_delete_occurrence_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let occurrence = OccurrenceId(1);
    let snapshot = app.document.current();
    let existing = snapshot.occurrence(occurrence).unwrap();
    let expected_definition = existing.definition_id();
    let expected_name = existing.name().to_owned();
    let expected_transform = existing.transform();
    let expected_parent = existing.parent();
    let expected_tag = existing.tag();
    let expected_visible = existing.visible();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteOccurrence;
    app.assistant_target_input = occurrence.0.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteOccurrence(occurrence));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::OccurrenceState {
            definition: expected_definition,
            name: expected_name.clone(),
            transform: expected_transform,
            parent: expected_parent,
            tag: expected_tag,
            visible: expected_visible,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().occurrence(occurrence).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.occurrence(occurrence).unwrap();
    assert_eq!(restored.definition_id(), expected_definition);
    assert_eq!(restored.name(), expected_name);
    assert_eq!(restored.transform(), expected_transform);
    assert_eq!(restored.parent(), expected_parent);
    assert_eq!(restored.tag(), expected_tag);
    assert_eq!(restored.visible(), expected_visible);
}

#[test]
fn assistant_create_definition_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateDefinition;
    app.assistant_target_input = definition.0.to_string();
    app.assistant_value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "Reviewed component".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateDefinition(definition));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Text("Reviewed component".to_owned())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .definition(definition)
            .unwrap()
            .name(),
        "Reviewed component"
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().definition(definition).is_none());
}

#[test]
fn assistant_create_group_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateGroup;
    app.assistant_target_input = group.0.to_string();
    app.assistant_value_input = String::new();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "Reviewed root group".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateGroup(group));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::GroupState {
            name: "Reviewed root group".to_owned(),
            transform: Transform::identity(),
            parent: None,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.group(group).unwrap();
    assert_eq!(created.name(), "Reviewed root group");
    assert_eq!(created.transform(), Transform::identity());
    assert_eq!(created.parent(), None);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().group(group).is_none());
}

#[test]
fn assistant_create_occurrence_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let occurrence = OccurrenceId(24);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateOccurrence;
    app.assistant_target_input = occurrence.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "1:Reviewed occurrence".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateOccurrence(occurrence));
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::OccurrenceState {
            definition: INITIAL_BOX_DEFINITION,
            name: "Reviewed occurrence".to_owned(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.occurrence(occurrence).unwrap();
    assert_eq!(created.definition_id(), INITIAL_BOX_DEFINITION);
    assert_eq!(created.name(), "Reviewed occurrence");
    assert_eq!(created.transform(), Transform::identity());
    assert_eq!(created.parent(), None);
    assert_eq!(created.tag(), None);
    assert!(created.visible());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().occurrence(occurrence).is_none());
}

#[test]
fn assistant_create_profile_feature_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let feature = FeatureId(24);
    let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    let feature_ids_before = app
        .document
        .current()
        .definition(INITIAL_BOX_DEFINITION)
        .unwrap()
        .feature_ids()
        .to_vec();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateProfileFeature;
    app.assistant_target_input = feature.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "1:Reviewed profile:0,0;20,0;20,10;0,10".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateProfileFeature(feature));
    assert_eq!(proposal.authoritative_writes().len(), 2);
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_core::document::AuthoritativeDependency::Feature(feature)
        })
        .unwrap();
    assert_eq!(feature_diff.before, ProposalValue::Missing);
    assert_eq!(
        feature_diff.after,
        ProposalValue::ProfileFeatureState {
            definition: INITIAL_BOX_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    let definition_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target
                == ketchup_core::document::AuthoritativeDependency::Definition(
                    INITIAL_BOX_DEFINITION,
                )
        })
        .unwrap();
    let mut feature_ids_after = feature_ids_before.clone();
    feature_ids_after.push(feature);
    assert_eq!(
        definition_diff.before,
        ProposalValue::DefinitionFeatures(feature_ids_before.clone())
    );
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionFeatures(feature_ids_after)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.feature(feature).unwrap();
    assert_eq!(created.definition_id(), INITIAL_BOX_DEFINITION);
    assert_eq!(created.name(), "Reviewed profile");
    assert!(matches!(
        created.kind(),
        FeatureKind::Profile { points_mm: created_points } if created_points == &points_mm
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().feature(feature).is_none());
    assert_eq!(
        app.document
            .current()
            .definition(INITIAL_BOX_DEFINITION)
            .unwrap()
            .feature_ids(),
        feature_ids_before
    );
}

#[test]
fn assistant_delete_profile_feature_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let feature = FeatureId(24);
    let points_mm = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: feature,
            definition_id: INITIAL_BOX_DEFINITION,
            name: "Reviewed profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: points_mm.clone(),
            },
        }]))
        .unwrap();
    let feature_ids_before = app
        .document
        .current()
        .definition(INITIAL_BOX_DEFINITION)
        .unwrap()
        .feature_ids()
        .to_vec();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteProfileFeature;
    app.assistant_target_input = feature.0.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteProfileFeature(feature));
    assert_eq!(proposal.authoritative_writes().len(), 2);
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_core::document::AuthoritativeDependency::Feature(feature)
        })
        .unwrap();
    assert_eq!(
        feature_diff.before,
        ProposalValue::ProfileFeatureState {
            definition: INITIAL_BOX_DEFINITION,
            name: "Reviewed profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(feature_diff.after, ProposalValue::Missing);
    let definition_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target
                == ketchup_core::document::AuthoritativeDependency::Definition(
                    INITIAL_BOX_DEFINITION,
                )
        })
        .unwrap();
    let mut feature_ids_after = feature_ids_before.clone();
    feature_ids_after.retain(|candidate| *candidate != feature);
    assert_eq!(
        definition_diff.before,
        ProposalValue::DefinitionFeatures(feature_ids_before.clone())
    );
    assert_eq!(
        definition_diff.after,
        ProposalValue::DefinitionFeatures(feature_ids_after)
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().feature(feature).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.feature(feature).unwrap();
    assert_eq!(restored.definition_id(), INITIAL_BOX_DEFINITION);
    assert_eq!(restored.name(), "Reviewed profile");
    assert_eq!(
        restored.kind(),
        &FeatureKind::Profile {
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(
        snapshot
            .definition(INITIAL_BOX_DEFINITION)
            .unwrap()
            .feature_ids(),
        feature_ids_before
    );
}

#[test]
fn assistant_create_evaluator_input_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = NodeId(99);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateEvaluatorInput;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "missing delimiter".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "Reviewed depth:42.5".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorInput(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorInputState {
            name: "Reviewed depth".to_owned(),
            dimension: Dimension::from_decimal("42.5").unwrap(),
            dependencies: Vec::new(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed depth");
    assert_eq!(
        created.dimension(),
        Some(&Dimension::from_decimal("42.5").unwrap())
    );
    assert!(created.dependencies().is_empty());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().evaluator_node(target).is_none());
}

#[test]
fn assistant_create_evaluator_expression_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "Reviewed source".to_owned(),
                dimension: Dimension::from_decimal("21").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let target = NodeId(100);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateEvaluatorExpression;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "missing delimiter".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "Reviewed double:$1 * 2".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreateEvaluatorExpression(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorExpressionState {
            name: "Reviewed double".to_owned(),
            expression: "$1 * 2".to_owned(),
            dependencies: vec![NodeId(1)],
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed double");
    assert_eq!(created.kind().source(), "$1 * 2");
    assert_eq!(created.dependencies(), &[NodeId(1)]);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().evaluator_node(target).is_none());
}

#[test]
fn assistant_create_evaluator_rule_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "Reviewed source".to_owned(),
                dimension: Dimension::from_decimal("21").unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let target = NodeId(100);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateEvaluatorRule;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "missing delimiter".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "Reviewed rule:$1 * 2".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateEvaluatorRule(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::EvaluatorRuleState {
            name: "Reviewed rule".to_owned(),
            expression: "$1 * 2".to_owned(),
            dependencies: vec![NodeId(1)],
            input_ports: Vec::new(),
            output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
            outputs: Vec::new(),
            override_parameters: Vec::new(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.evaluator_node(target).unwrap();
    assert_eq!(created.name(), "Reviewed rule");
    assert_eq!(created.kind().source(), "$1 * 2");
    assert_eq!(created.dependencies(), &[NodeId(1)]);
    assert!(created.input_ports().is_empty());
    assert_eq!(
        created.output_ports(),
        &[ketchup_core::document::PortSpec::number("result").unwrap()]
    );
    assert!(created.allowed_parameters().is_empty());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().evaluator_node(target).is_none());
}

#[test]
fn assistant_create_rule_override_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(101);
    let target = 102;
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "Reviewed override source".to_owned(),
            expression: "1".to_owned(),
            input_ports: Vec::new(),
            output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
            outputs: vec![
                RuleOutput::new(
                    SlotSegment::new(rule, "result", "left").unwrap(),
                    Vec::new(),
                )
                .unwrap(),
            ],
            override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
        }]))
        .unwrap();
    let identity = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateRuleOverride;
    app.assistant_target_input = target.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "101:result:left:offset:2.5".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateRuleOverride(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::RuleOverrideState {
            target: identity.clone(),
            parameter: "offset".to_owned(),
            value: 2.5,
            health: SlotResolution::Resolved,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let snapshot = app.document.current();
    let created = snapshot.override_by_id(target).unwrap();
    assert_eq!(created.target, identity);
    assert_eq!(created.parameter, "offset");
    assert_eq!(created.value(), 2.5);
    assert_eq!(created.health, SlotResolution::Resolved);
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().override_by_id(target).is_none());
}

#[test]
fn assistant_create_feature_parameter_binding_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(200);
    let profile = FeatureId(201);
    let feature = FeatureId(202);
    let rule = NodeId(203);
    let target =
        FeatureParameterTarget::new(feature, "height", ParameterValueType::Length).unwrap();
    let derived_from = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Bound box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Bound profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: definition,
                name: "Bound extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Binding source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateFeatureParameterBinding;
    app.assistant_target_input = feature.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "height:203:result:left".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreateFeatureParameterBinding(target.clone())
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::FeatureParameterBindingState {
            target: target.clone(),
            derived_from: derived_from.clone(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .unwrap()
            .derived_from,
        derived_from
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .is_none()
    );
}

#[test]
fn assistant_delete_feature_parameter_binding_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(204);
    let profile = FeatureId(205);
    let feature = FeatureId(206);
    let rule = NodeId(207);
    let target =
        FeatureParameterTarget::new(feature, "height", ParameterValueType::Length).unwrap();
    let derived_from = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Bound box deletion".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Bound profile deletion".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: definition,
                name: "Bound extrusion deletion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Binding deletion source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target: target.clone(),
                    derived_from: derived_from.clone(),
                },
            ),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteFeatureParameterBinding;
    app.assistant_target_input = feature.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "height".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeleteFeatureParameterBinding(target.clone())
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::FeatureParameterBindingState {
            target: target.clone(),
            derived_from: derived_from.clone(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .is_none()
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .feature_parameter_binding(&target)
            .unwrap()
            .derived_from,
        derived_from
    );
}

#[test]
fn assistant_recompute_feature_parameter_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(208);
    let profile = FeatureId(209);
    let feature = FeatureId(210);
    let rule = NodeId(211);
    let target =
        FeatureParameterTarget::new(feature, "height", ParameterValueType::Length).unwrap();
    let derived_from = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "height").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Recomputed box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Recomputed profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: feature,
                definition_id: definition,
                name: "Recomputed extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Recompute source".to_owned(),
                expression: "42".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "height").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: Vec::new(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(
                ketchup_core::document::FeatureParameterBinding {
                    target: target.clone(),
                    derived_from,
                },
            ),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::RecomputeFeatureParameter;
    app.assistant_target_input = feature.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "height".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::RecomputeFeatureParameter(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Dimension(Dimension::from_decimal("20").unwrap())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Dimension(Dimension::from_decimal("42").unwrap())
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(matches!(
        app.document.current().feature(feature).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "42" && height.millimetres() == 42.0
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(feature).unwrap().kind(),
        FeatureKind::Extrusion { height, .. }
            if height.source_token() == "20" && height.millimetres() == 20.0
    ));
}

#[test]
fn assistant_clone_profile_definition_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let source_definition = DefinitionId(300);
    let source_feature = FeatureId(301);
    let occurrence = OccurrenceId(302);
    let new_definition = DefinitionId(303);
    let new_feature = FeatureId(304);
    let points_mm = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: source_definition,
                name: "Clone source".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: source_feature,
                definition_id: source_definition,
                name: "Source profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: points_mm.clone(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: source_definition,
                name: "Clone occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CloneProfileDefinitionAndRepoint;
    app.assistant_target_input = occurrence.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = format!(
        "{}:{}:{}:{}:Independent profile",
        source_definition.0, source_feature.0, new_definition.0, new_feature.0
    );
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CloneProfileDefinitionAndRepoint(occurrence)
    );
    assert_eq!(proposal.authoritative_writes().len(), 3);
    let feature_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_core::document::AuthoritativeDependency::Feature(new_feature)
        })
        .unwrap();
    assert_eq!(feature_diff.before, ProposalValue::Missing);
    assert_eq!(
        feature_diff.after,
        ProposalValue::ProfileFeatureState {
            definition: new_definition,
            name: "Source profile".to_owned(),
            points_mm: points_mm.clone(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .occurrence(occurrence)
            .unwrap()
            .definition_id(),
        new_definition
    );
    assert!(matches!(
        app.document.current().feature(new_feature).unwrap().kind(),
        FeatureKind::Profile { points_mm: cloned } if cloned == &points_mm
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .occurrence(occurrence)
            .unwrap()
            .definition_id(),
        source_definition
    );
    assert!(app.document.current().definition(new_definition).is_none());
    assert!(app.document.current().feature(new_feature).is_none());
}

#[test]
fn assistant_convert_empty_group_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let group = GroupId(300);
    let new_definition = DefinitionId(301);
    let new_occurrence = OccurrenceId(302);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
            id: group,
            name: "Reviewed empty group".to_owned(),
            transform: Transform::from_translation(1.0, 2.0, 3.0).unwrap(),
            parent: None,
        }]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::ConvertEmptyGroupToComponent;
    app.assistant_target_input = group.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = format!(
        "{}:{}:Reviewed component",
        new_definition.0, new_occurrence.0
    );
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::ConvertEmptyGroupToComponent(group)
    );
    assert_eq!(proposal.authoritative_writes().len(), 3);
    let group_diff = proposal
        .authoritative_diff()
        .iter()
        .find(|entry| {
            entry.target == ketchup_core::document::AuthoritativeDependency::GroupSubtree(group)
        })
        .unwrap();
    assert!(matches!(
        group_diff.before,
        ProposalValue::GroupState { ref name, .. } if name == "Reviewed empty group"
    ));
    assert_eq!(group_diff.after, ProposalValue::Missing);
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().group(group).is_none());
    assert!(app.document.current().definition(new_definition).is_some());
    assert_eq!(
        app.document
            .current()
            .occurrence(new_occurrence)
            .unwrap()
            .transform(),
        Transform::from_translation(1.0, 2.0, 3.0).unwrap()
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().group(group).is_some());
    assert!(app.document.current().definition(new_definition).is_none());
    assert!(app.document.current().occurrence(new_occurrence).is_none());
}

#[test]
fn assistant_create_joint_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(500);
    let target = JointId(222);
    let output =
        |key| RuleOutput::new(SlotSegment::new(rule, "result", key).unwrap(), Vec::new()).unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateRuleNode {
            id: rule,
            name: "joint participants".to_owned(),
            expression: "1".to_owned(),
            input_ports: Vec::new(),
            output_ports: vec![PortSpec::number("result").unwrap()],
            outputs: vec![output("left"), output("right")],
            override_parameters: Vec::new(),
        }]))
        .unwrap();
    let participant = |key| {
        DerivedIdentity::new(
            rule,
            SlotPath::new(vec![SlotSegment::new(rule, "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateJoint;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "500,result,left:500,result,right:1,2,3:4,5,6".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateJoint(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::JointState {
            participant_a: participant("left"),
            participant_b: participant("right"),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let joint = ketchup_core::prismatic::CanonicalJoint::new(
        target,
        participant("left"),
        participant("right"),
        ketchup_core::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
    )
    .unwrap();
    assert_eq!(app.document.current().joint(target), Some(&joint));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().joint(target).is_none());
}

#[test]
fn assistant_delete_joint_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = JointId(212);
    let participant = |key| {
        DerivedIdentity::new(
            NodeId(213),
            SlotPath::new(vec![SlotSegment::new(NodeId(213), "result", key).unwrap()]).unwrap(),
        )
        .unwrap()
    };
    let joint = ketchup_core::prismatic::CanonicalJoint::new(
        target,
        participant("left"),
        participant("right"),
        ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertJoint(
            joint.clone(),
        )]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteJoint;
    app.assistant_target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteJoint(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::JointState {
            participant_a: joint.participant_a().clone(),
            participant_b: joint.participant_b().clone(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().joint(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().joint(target), Some(&joint));
}

#[test]
fn assistant_create_space_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = SpaceId(219);
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateSpace;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "maintenance access:1,2,3:4,5,6".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateSpace(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::SpaceState {
            purpose: "maintenance access".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            adjacent_to: Vec::new(),
            accessible_to: Vec::new(),
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let space = ketchup_core::space::CanonicalSpace::new(
        target,
        "maintenance access",
        ketchup_core::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(app.document.current().space(target), Some(&space));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().space(target).is_none());
}

#[test]
fn assistant_create_clearance_volume_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let owner = SpaceId(220);
    let target = ClearanceVolumeId(221);
    let space = ketchup_core::space::CanonicalSpace::new(
        owner,
        "equipment",
        ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [5.0, 5.0, 5.0]).unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            space,
        )]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreateClearanceVolume;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "220:maintenance envelope:1,2,3:4,5,6:0.01:required".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::CreateClearanceVolume(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::ClearanceVolumeState {
            owner: ClearanceOwner::Space(owner),
            reason: "maintenance envelope".to_owned(),
            volume_min: [1.0, 2.0, 3.0],
            volume_max: [4.0, 5.0, 6.0],
            coordinate_frame: ketchup_core::space::ClearanceCoordinateFrame::World,
            tolerance_mm: 0.01,
            severity: ClearanceSeverity::Required,
            derived_from: None,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let clearance = ketchup_core::space::CanonicalClearanceVolume::new(
        target,
        ClearanceOwner::Space(owner),
        "maintenance envelope",
        ketchup_core::prismatic::Aabb::bounded_volume([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
        TolerancePolicy::new(0.01).unwrap(),
        ClearanceSeverity::Required,
        None,
    )
    .unwrap();
    assert_eq!(
        app.document.current().clearance_volume(target),
        Some(&clearance)
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(app.document.current().clearance_volume(target).is_none());
}

#[test]
fn assistant_delete_space_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = SpaceId(214);
    let space = ketchup_core::space::CanonicalSpace::new(
        target,
        "maintenance access",
        ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]).unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertSpace(
            space.clone(),
        )]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteSpace;
    app.assistant_target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteSpace(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::SpaceState {
            purpose: "maintenance access".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            adjacent_to: Vec::new(),
            accessible_to: Vec::new(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().space(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().space(target), Some(&space));
}

#[test]
fn assistant_delete_clearance_volume_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let owner = SpaceId(215);
    let target = ClearanceVolumeId(216);
    let space = ketchup_core::space::CanonicalSpace::new(
        owner,
        "equipment",
        ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [5.0, 5.0, 5.0]).unwrap(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let clearance = ketchup_core::space::CanonicalClearanceVolume::new(
        target,
        ketchup_core::space::ClearanceOwner::Space(owner),
        "maintenance envelope",
        ketchup_core::prismatic::Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]).unwrap(),
        ketchup_core::prismatic::TolerancePolicy::new(0.01).unwrap(),
        ketchup_core::space::ClearanceSeverity::Required,
        None,
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertSpace(space),
            CanonicalCommand::UpsertClearanceVolume(clearance.clone()),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteClearanceVolume;
    app.assistant_target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteClearanceVolume(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::ClearanceVolumeState {
            owner: ketchup_core::space::ClearanceOwner::Space(owner),
            reason: "maintenance envelope".to_owned(),
            volume_min: [0.0, 0.0, 0.0],
            volume_max: [1.0, 2.0, 3.0],
            coordinate_frame: ketchup_core::space::ClearanceCoordinateFrame::World,
            tolerance_mm: 0.01,
            severity: ketchup_core::space::ClearanceSeverity::Required,
            derived_from: None,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().clearance_volume(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().clearance_volume(target),
        Some(&clearance)
    );
}

#[test]
fn assistant_delete_persistent_dimension_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = PersistentDimensionId(217);
    let dimension_target = PersistentDimensionTarget::FeatureParameter(
        FeatureParameterTarget::new(FeatureId(2), "bounds.width", ParameterValueType::Length)
            .unwrap(),
    );
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
    let dimension = PersistentDimension::new(
        target,
        "Cabinet width",
        dimension_target.clone(),
        presentation,
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertPersistentDimension(dimension.clone()),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeletePersistentDimension;
    app.assistant_target_input = target.0.to_string();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::DeletePersistentDimension(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::PersistentDimensionState {
            name: "Cabinet width".to_owned(),
            target: dimension_target,
            presentation,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(
        app.document
            .current()
            .persistent_dimension(target)
            .is_none()
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document.current().persistent_dimension(target),
        Some(&dimension)
    );
}

#[test]
fn assistant_create_persistent_dimension_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let target = PersistentDimensionId(218);
    let dimension_target =
        FeatureParameterTarget::new(FeatureId(2), "height", ParameterValueType::Length).unwrap();
    let presentation = DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CreatePersistentDimension;
    app.assistant_target_input = target.0.to_string();
    app.assistant_value_input = "invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert_eq!(app.canonical_digest(), digest_before);

    app.assistant_value_input = "Reviewed height:2:height:cm:2".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::CreatePersistentDimension(target)
    );
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Missing
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::PersistentDimensionState {
            name: "Reviewed height".to_owned(),
            target: PersistentDimensionTarget::FeatureParameter(dimension_target.clone()),
            presentation,
        }
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    let dimension = PersistentDimension::new(
        target,
        "Reviewed height",
        PersistentDimensionTarget::FeatureParameter(dimension_target),
        presentation,
    )
    .unwrap();
    assert_eq!(
        app.document.current().persistent_dimension(target),
        Some(&dimension)
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert!(
        app.document
            .current()
            .persistent_dimension(target)
            .is_none()
    );
}

#[test]
fn assistant_delete_rule_override_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let rule = NodeId(101);
    let target = 102;
    let identity = DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", "left").unwrap()]).unwrap(),
    )
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateRuleNode {
                id: rule,
                name: "Reviewed override source".to_owned(),
                expression: "1".to_owned(),
                input_ports: Vec::new(),
                output_ports: vec![ketchup_core::document::PortSpec::number("result").unwrap()],
                outputs: vec![
                    RuleOutput::new(
                        SlotSegment::new(rule, "result", "left").unwrap(),
                        Vec::new(),
                    )
                    .unwrap(),
                ],
                override_parameters: vec![OverrideParameterSpec::replace("offset").unwrap()],
            },
            CanonicalCommand::UpsertOverride(
                ketchup_core::document::CanonicalOverride::new(
                    target,
                    identity.clone(),
                    "offset",
                    2.5,
                    SlotResolution::Resolved,
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteRuleOverride;
    app.assistant_target_input = target.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteRuleOverride(target));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::RuleOverrideState {
            target: identity.clone(),
            parameter: "offset".to_owned(),
            value: 2.5,
            health: SlotResolution::Resolved,
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().override_by_id(target).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let restored = app
        .document
        .current()
        .override_by_id(target)
        .unwrap()
        .clone();
    assert_eq!(restored.target, identity);
    assert_eq!(restored.parameter, "offset");
    assert_eq!(restored.value(), 2.5);
    assert_eq!(restored.health, SlotResolution::Resolved);
}

#[test]
fn assistant_delete_definition_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let definition = DefinitionId(99);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Reviewed empty definition".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::DeleteDefinition;
    app.assistant_target_input = definition.0.to_string();
    app.assistant_value_input.clear();

    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(proposal.goal(), ProposalGoal::DeleteDefinition(definition));
    assert_eq!(proposal.authoritative_writes().len(), 1);
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::DefinitionState {
            name: "Reviewed empty definition".to_owned(),
            feature_ids: Vec::new(),
            local_occurrence_ids: Vec::new(),
            local_group_ids: Vec::new(),
        }
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Missing
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert!(app.document.current().definition(definition).is_none());
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    let snapshot = app.document.current();
    let restored = snapshot.definition(definition).unwrap();
    assert_eq!(restored.name(), "Reviewed empty definition");
    assert!(restored.feature_ids().is_empty());
}

#[test]
fn assistant_collection_membership_review_is_typed_observational_and_undoable() {
    let mut app = KetchupApp::new();
    let collection = CollectionId(12);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateCollection {
                id: collection,
                name: "Selection set".to_owned(),
            },
        ]))
        .unwrap();
    let revision_before = app.document_revision();
    let digest_before = app.canonical_digest();
    let undo_before = app.document.visible_undo_steps();
    app.assistant_intent_kind = AssistantIntentKind::CollectionOccurrences;
    app.assistant_target_input = collection.0.to_string();
    app.assistant_value_input = "1, invalid".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "1, 1".to_owned();
    assert!(!app.prepare_assistant_from_inputs());
    assert!(app.assistant_proposal().is_none());
    assert_eq!(app.document_revision(), revision_before);

    app.assistant_value_input = "1".to_owned();
    assert!(app.prepare_assistant_from_inputs());
    let proposal = app.assistant_proposal().unwrap();
    assert_eq!(
        proposal.goal(),
        ProposalGoal::SetCollectionOccurrences(collection)
    );
    assert_eq!(
        proposal.authoritative_diff()[0].before,
        ProposalValue::Occurrences(Vec::new())
    );
    assert_eq!(
        proposal.authoritative_diff()[0].after,
        ProposalValue::Occurrences(vec![OccurrenceId(1)])
    );
    assert_eq!(app.document_revision(), revision_before);
    assert_eq!(app.canonical_digest(), digest_before);
    assert_eq!(app.document.visible_undo_steps(), undo_before);

    assert!(app.confirm_assistant_proposal());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .collect::<Vec<_>>(),
        vec![OccurrenceId(1)]
    );
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert!(app.undo());
    assert_eq!(
        app.document
            .current()
            .collection(collection)
            .unwrap()
            .occurrence_ids()
            .count(),
        0
    );
}

#[test]
fn push_pull_uses_the_projected_feature_pair_and_preserves_local_profile_origin() {
    let mut app = KetchupApp::new();
    let offset_points = vec![[10.0, 20.0], [110.0, 20.0], [110.0, 80.0], [10.0, 80.0]];
    let unrelated_points = vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]];
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: FeatureId(1),
                points_mm: offset_points,
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: INITIAL_BOX_DEFINITION,
                name: "Unrelated profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: unrelated_points.clone(),
                },
            },
        ]))
        .unwrap();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::X,
            side: Side::Maximum,
        },
    };
    app.selection.primary = Some(selection.clone());
    let item = app.active_boxes()[0].clone();
    assert_eq!(item.profile_feature_id, FeatureId(1));
    assert_eq!(item.extrusion_feature_id, Some(FeatureId(2)));
    assert_eq!(item.origin_mm, Vec3::new(10.0, 20.0, 0.0));
    assert!(
        push_pull_batch(
            &app.document.current(),
            &SelectionId {
                definition_id: DefinitionId(999),
                ..selection
            },
            &item,
            None,
            50.0,
            150.0,
            "150".to_owned(),
        )
        .is_none()
    );

    app.set_push_pull_distance_input("50");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    let snapshot = app.document.current();
    let FeatureKind::Profile { points_mm } = snapshot.feature(FeatureId(1)).unwrap().kind() else {
        panic!("linked profile must remain a profile");
    };
    assert_eq!(
        points_mm,
        &vec![[10.0, 20.0], [160.0, 20.0], [160.0, 80.0], [10.0, 80.0]]
    );
    let FeatureKind::Profile { points_mm } = snapshot.feature(FeatureId(3)).unwrap().kind() else {
        panic!("unrelated feature must remain a profile");
    };
    assert_eq!(points_mm, &unrelated_points);
}

#[test]
fn push_pull_preview_fails_closed_after_the_source_revision_changes() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    let current = app.active_boxes()[0].clone();
    assert!(!app.has_preview());
    assert_eq!(app.render_box(current.clone()), current);
    assert!(!app.confirm_preview());
}

#[test]
fn linear_pattern_preview_adds_virtual_viewport_occurrences_only() {
    let mut app = KetchupApp::new();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    assert!(app.preview_linear_pattern(OccurrenceId(1), Axis::Z, 50.0, 4));

    let snapshot = app.document.current();
    let exact_projection = app.exact_projection(&snapshot);
    let boxes = app.viewport_boxes(&exact_projection);
    assert_eq!(boxes.len(), 4);
    assert_eq!(boxes[3].origin_mm, Vec3::new(0.0, 0.0, 150.0));
    assert_eq!(app.active_box_count(), 1);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
}

#[test]
fn occurrence_alignment_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    assert!(app.copy_selected(Vec3::new(300.0, 300.0, 100.0)));
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let source = app.occurrence_alignment_source_plan().unwrap();
    let plan = app
        .occurrence_alignment_plan(&source, Axis::Z, AlignMode::Center)
        .unwrap();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_command = plan.clone();
    tampered_command.command = CanonicalCommand::SetOccurrenceTransform {
        id: source.reference_id,
        transform: source.reference_transform,
    };
    assert!(!app.preview_occurrence_alignment_plan(tampered_command));
    let mut tampered_source = plan.clone();
    tampered_source.source.moving_visible = !tampered_source.source.moving_visible;
    assert!(!app.preview_occurrence_alignment_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    app.begin_occurrence_align();
    app.pending_occurrence_align.as_mut().unwrap().axis = Axis::Z;
    assert!(app.preview_pending_occurrence_align());
    let preview = app.occurrence_operation_preview.as_mut().unwrap();
    preview.batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: source.reference_id,
    }]);
    preview.command_digest = preview.batch.digest();
    assert!(!app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.preview_pending_occurrence_align());
    app.occurrence_operation_preview
        .as_mut()
        .unwrap()
        .boxes
        .get_mut(&source.moving_id)
        .unwrap()
        .origin_mm
        .x += 1.0;
    assert!(!app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.preview_pending_occurrence_align());
    let Some(OccurrenceCanonicalPreviewPlan::Alignment(sealed_plan)) = app
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .canonical_plan
        .as_mut()
    else {
        panic!("alignment preview seals its typed plan");
    };
    sealed_plan.mode = AlignMode::Maximum;
    assert!(!app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.preview_pending_occurrence_align());
    app.pending_occurrence_align
        .as_mut()
        .unwrap()
        .preview_plan
        .as_mut()
        .unwrap()
        .command = CanonicalCommand::SetOccurrenceTransform {
        id: source.reference_id,
        transform: source.reference_transform,
    };
    assert!(!app.confirm_occurrence_align());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    app.pending_occurrence_align.as_mut().unwrap().preview_plan = Some(plan.clone());
    assert!(app.confirm_occurrence_align());
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let aligned_digest = app.canonical_digest();
    assert!(!app.preview_occurrence_alignment_plan(plan));
    assert_eq!(app.canonical_digest(), aligned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), aligned_digest);
}

#[test]
fn occurrence_distribution_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    assert!(app.copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    assert!(app.copy_selected(Vec3::new(400.0, 0.0, 0.0)));
    assert!(app.copy_selected(Vec3::new(400.0, 0.0, 0.0)));
    assert!(app.select_all());
    let source = app.occurrence_distribution_source_plan().unwrap();
    let plan = app
        .occurrence_distribution_plan(&source, Axis::X, DistributionMode::Centers)
        .unwrap();
    assert_eq!(
        plan.ordered_occurrence_ids,
        vec![
            OccurrenceId(1),
            OccurrenceId(2),
            OccurrenceId(3),
            OccurrenceId(4)
        ]
    );
    assert_eq!(plan.source_coordinates_mm, vec![50.0, 150.0, 550.0, 950.0]);
    assert_eq!(plan.target_coordinates_mm, vec![50.0, 350.0, 650.0, 950.0]);
    assert_eq!(plan.spacing_mm, 300.0);
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.preview_occurrence_distribution_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    let item = tampered_source
        .source
        .occurrences
        .get_mut(&OccurrenceId(2))
        .unwrap();
    item.visible = !item.visible;
    assert!(!app.preview_occurrence_distribution_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    app.begin_occurrence_distribution();
    assert!(app.preview_pending_occurrence_distribution());
    app.pending_occurrence_distribution
        .as_mut()
        .unwrap()
        .preview_plan
        .as_mut()
        .unwrap()
        .target_coordinates_mm[1] += 1.0;
    assert!(!app.confirm_occurrence_distribution());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    app.pending_occurrence_distribution
        .as_mut()
        .unwrap()
        .preview_plan = Some(plan.clone());
    assert!(app.confirm_occurrence_distribution());
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let distributed_digest = app.canonical_digest();
    assert!(!app.preview_occurrence_distribution_plan(plan));
    assert_eq!(app.canonical_digest(), distributed_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), distributed_digest);
}

#[test]
fn linear_pattern_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let source = app.linear_pattern_source_plan().unwrap();
    let plan = app.linear_pattern_plan(&source, Axis::X, 25.0, 3).unwrap();
    assert!(app.preview_linear_pattern_plan(plan.clone()));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.apply_linear_pattern_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    tampered_source.source.source_primary = None;
    assert!(!app.apply_linear_pattern_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.apply_linear_pattern_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.active_box_count(), 3);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let patterned_digest = app.canonical_digest();
    assert!(!app.apply_linear_pattern_plan(plan));
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.active_box_count(), 3);
}

#[test]
fn rectangular_pattern_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let source = app.rectangular_pattern_source_plan().unwrap();
    let spec = RectangularPatternSpec {
        primary_axis: Axis::X,
        primary_spacing_mm: 25.0,
        primary_count: 2,
        secondary_axis: Axis::Y,
        secondary_spacing_mm: 30.0,
        secondary_count: 2,
    };
    let plan = app.rectangular_pattern_plan(&source, spec).unwrap();
    assert!(app.preview_rectangular_pattern_plan(plan.clone()));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.apply_rectangular_pattern_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    tampered_source.source.source_primary = None;
    assert!(!app.apply_rectangular_pattern_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.apply_rectangular_pattern_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.active_box_count(), 4);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let patterned_digest = app.canonical_digest();
    assert!(!app.apply_rectangular_pattern_plan(plan));
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.active_box_count(), 4);
}

#[test]
fn circular_pattern_plan_rejects_tamper_and_replay_atomically() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let source = app.circular_pattern_source_plan().unwrap();
    let plan = app
        .circular_pattern_plan(&source, Axis::Z, Vec3::new(100.0, 0.0, 0.0), 90.0, 4)
        .unwrap();
    assert!(app.preview_circular_pattern_plan(plan.clone()));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    let mut tampered_commands = plan.clone();
    tampered_commands.commands.reverse();
    assert!(!app.apply_circular_pattern_plan(tampered_commands));
    let mut tampered_source = plan.clone();
    tampered_source.source.source_primary = None;
    assert!(!app.apply_circular_pattern_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.apply_circular_pattern_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.active_box_count(), 4);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let patterned_digest = app.canonical_digest();
    assert!(!app.apply_circular_pattern_plan(plan));
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), patterned_digest);
    assert_eq!(app.active_box_count(), 4);
}

#[test]
fn rectangle_sketch_creates_a_profile_then_push_pull_adds_the_extrusion() {
    let mut app = KetchupApp::new();

    assert!(
        app.complete_rectangle_sketch(Vec3::new(40.0, 25.0, 0.0), Vec3::new(-10.0, -5.0, 0.0),)
    );
    assert_eq!(app.active_box_count(), 2);
    let profile = app.active_boxes()[1].clone();
    assert_eq!(profile.origin_mm, Vec3::new(-10.0, -5.0, 0.0));
    assert_eq!(profile.size_mm, Vec3::new(50.0, 30.0, 0.0));
    assert_eq!(profile.extrusion_feature_id, None);
    let profile_digest = app.canonical_digest();

    app.set_push_pull_distance_input("30");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    let solid = app.active_boxes()[1].clone();
    assert_eq!(solid.size_mm, Vec3::new(50.0, 30.0, 30.0));
    assert!(solid.extrusion_feature_id.is_some());

    assert!(app.undo());
    assert_eq!(app.canonical_digest(), profile_digest);
    assert_eq!(app.active_boxes()[1].size_mm.z, 0.0);
    assert!(app.redo());
    assert_eq!(app.active_boxes()[1].size_mm.z, 30.0);
}

#[test]
fn contained_slanted_polygon_solid_tools_round_trip_atomically() {
    fn add_polygon_tool(app: &mut KetchupApp, points: &[[f64; 2]]) {
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
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Polygon tool".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(2),
                    name: "Slanted containing profile".to_owned(),
                    kind: FeatureKind::SegmentProfile {
                        segments,
                        closed: true,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(2),
                    name: "Polygon tool extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(3),
                        height: Dimension::new("20", 20.0).unwrap(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(2),
                    name: "Polygon tool occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        app.document.discard_history_before_current();
    }

    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };

    let mut app = KetchupApp::new();
    add_polygon_tool(
        &mut app,
        &[[-20.0, -10.0], [110.0, -15.0], [125.0, 70.0], [-15.0, 80.0]],
    );
    app.active_tool = ActiveTool::SolidUnion;
    let before_digest = app.canonical_digest();
    let before_revision = app.document_revision();
    let before_undo = app.document.visible_undo_steps();
    app.solid_tool_target = Some(target.clone());
    assert!(app.prepare_solid_tool_preview(tool.clone(), false));
    assert!(app.has_occurrence_operation_preview());
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.document.visible_undo_steps(), before_undo);
    assert_eq!(
        app.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(-20.0, -15.0, 0.0), Vec3::new(145.0, 95.0, 20.0)))
    );
    app.clear_ephemeral_edit_state();
    assert!(!app.has_occurrence_operation_preview());
    assert_eq!(app.canonical_digest(), before_digest);

    app.active_tool = ActiveTool::SolidUnion;
    app.solid_tool_target = Some(target);
    assert!(app.prepare_solid_tool_preview(tool, false));
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.document.visible_undo_steps(), before_undo + 1);
    let committed = app.document.current();
    let result_definition = committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(committed.occurrence(OccurrenceId(2)).is_none());
    let polygon_result_feature_id = *committed
        .definition(result_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&committed, result_definition, polygon_result_feature_id)
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    let committed_digest = committed.canonical_digest();
    let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&committed))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(
        ExactBRepGraph::from_snapshot(&reopened, result_definition, polygon_result_feature_id)
            .is_ok()
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed_digest);

    let mut partial = KetchupApp::new();
    add_polygon_tool(
        &mut partial,
        &[[20.0, 15.0], [115.0, 12.0], [110.0, 45.0], [18.0, 42.0]],
    );
    partial.active_tool = ActiveTool::SolidUnion;
    let partial_digest = partial.canonical_digest();
    partial.solid_tool_target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(partial.prepare_solid_tool_preview(
        SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    assert_eq!(partial.canonical_digest(), partial_digest);
    assert!(partial.has_occurrence_operation_preview());

    let intersect_target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let intersect_tool = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let mut intersection = KetchupApp::new();
    add_polygon_tool(
        &mut intersection,
        &[
            [12.0, 10.0],
            [70.0, 8.0],
            [88.0, 40.0],
            [55.0, 52.0],
            [15.0, 45.0],
        ],
    );
    intersection.active_tool = ActiveTool::SolidIntersect;
    let intersect_before_digest = intersection.canonical_digest();
    let intersect_before_revision = intersection.document_revision();
    let intersect_before_undo = intersection.document.visible_undo_steps();
    intersection.solid_tool_target = Some(intersect_target.clone());
    assert!(intersection.prepare_solid_tool_preview(intersect_tool.clone(), false));
    assert!(intersection.has_occurrence_operation_preview());
    assert_eq!(intersection.canonical_digest(), intersect_before_digest);
    assert_eq!(intersection.document_revision(), intersect_before_revision);
    assert_eq!(
        intersection.document.visible_undo_steps(),
        intersect_before_undo
    );
    assert_eq!(
        intersection.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(12.0, 8.0, 0.0), Vec3::new(76.0, 44.0, 20.0)))
    );
    intersection.clear_ephemeral_edit_state();
    assert!(!intersection.has_occurrence_operation_preview());
    assert_eq!(intersection.canonical_digest(), intersect_before_digest);

    intersection.active_tool = ActiveTool::SolidIntersect;
    intersection.solid_tool_target = Some(intersect_target);
    assert!(intersection.prepare_solid_tool_preview(intersect_tool, false));
    assert!(intersection.confirm_occurrence_operation_preview());
    assert_eq!(
        intersection.document.visible_undo_steps(),
        intersect_before_undo + 1
    );
    let intersected = intersection.document.current();
    let intersect_definition = intersected
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(intersected.occurrence(OccurrenceId(2)).is_none());
    let intersect_feature_id = *intersected
        .definition(intersect_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let intersect_graph =
        ExactBRepGraph::from_snapshot(&intersected, intersect_definition, intersect_feature_id)
            .unwrap();
    assert!(intersect_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
    let intersect_digest = intersected.canonical_digest();
    let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&intersected))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), intersect_digest);
    assert!(
        ExactBRepGraph::from_snapshot(&reopened, intersect_definition, intersect_feature_id,)
            .is_ok()
    );
    assert!(intersection.undo());
    assert_eq!(intersection.canonical_digest(), intersect_before_digest);
    assert!(intersection.redo());
    assert_eq!(intersection.canonical_digest(), intersect_digest);

    let mut crossing = KetchupApp::new();
    add_polygon_tool(
        &mut crossing,
        &[[20.0, 10.0], [105.0, 8.0], [80.0, 50.0], [15.0, 45.0]],
    );
    crossing.active_tool = ActiveTool::SolidIntersect;
    let crossing_digest = crossing.canonical_digest();
    crossing.solid_tool_target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(crossing.prepare_solid_tool_preview(
        SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    assert_eq!(crossing.canonical_digest(), crossing_digest);
    assert!(crossing.has_occurrence_operation_preview());

    let mut split = KetchupApp::new();
    add_polygon_tool(
        &mut split,
        &[
            [12.0, 10.0],
            [70.0, 8.0],
            [88.0, 40.0],
            [55.0, 52.0],
            [15.0, 45.0],
        ],
    );
    split.active_tool = ActiveTool::SolidSplit;
    let split_before_digest = split.canonical_digest();
    let split_before_revision = split.document_revision();
    let split_before_undo = split.document.visible_undo_steps();
    split.solid_tool_target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    let split_tool = SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    assert!(split.prepare_solid_tool_preview(split_tool.clone(), false));
    assert!(split.has_occurrence_operation_preview());
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert_eq!(split.document_revision(), split_before_revision);
    assert_eq!(split.document.visible_undo_steps(), split_before_undo);
    assert_eq!(
        split.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::ZERO, Vec3::new(100.0, 60.0, 20.0)))
    );
    split.clear_ephemeral_edit_state();
    assert!(!split.has_occurrence_operation_preview());
    assert_eq!(split.canonical_digest(), split_before_digest);

    split.active_tool = ActiveTool::SolidSplit;
    split.solid_tool_target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(split.prepare_solid_tool_preview(split_tool, false));
    assert!(split.confirm_occurrence_operation_preview());
    assert_eq!(split.document.visible_undo_steps(), split_before_undo + 1);
    let split_snapshot = split.document.current();
    let split_definition = split_snapshot
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(split_snapshot.occurrence(OccurrenceId(2)).is_some());
    let split_feature_id = *split_snapshot
        .definition(split_definition)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let split_graph =
        ExactBRepGraph::from_snapshot(&split_snapshot, split_definition, split_feature_id).unwrap();
    assert!(split_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Split,
            ..
        }
    )));
    let split_digest = split_snapshot.canonical_digest();
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&split_snapshot))
            .unwrap()
            .snapshot();
    assert_eq!(reopened.canonical_digest(), split_digest);
    assert!(ExactBRepGraph::from_snapshot(&reopened, split_definition, split_feature_id).is_ok());
    assert!(split.undo());
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert!(split.redo());
    assert_eq!(split.canonical_digest(), split_digest);

    let mut boundary_touching = KetchupApp::new();
    add_polygon_tool(
        &mut boundary_touching,
        &[[20.0, 10.0], [100.0, 8.0], [80.0, 50.0], [15.0, 45.0]],
    );
    boundary_touching.active_tool = ActiveTool::SolidSplit;
    let boundary_digest = boundary_touching.canonical_digest();
    boundary_touching.solid_tool_target = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(boundary_touching.prepare_solid_tool_preview(
        SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    assert_eq!(boundary_touching.canonical_digest(), boundary_digest);
    assert!(boundary_touching.has_occurrence_operation_preview());
}

#[test]
fn contained_circle_subtract_intersect_split_and_containing_union_round_trip_atomically() {
    fn app_with_circle_tool(center: [f64; 2], radius: f64) -> KetchupApp {
        let mut app = KetchupApp::new();
        let left = [center[0] - radius, center[1]];
        let right = [center[0] + radius, center[1]];
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Circle tool".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(2),
                    name: "Circle profile".to_owned(),
                    kind: FeatureKind::SegmentProfile {
                        segments: vec![
                            ProfileSegment::CircularArc {
                                start_mm: left,
                                end_mm: right,
                                center_mm: center,
                                clockwise: false,
                            },
                            ProfileSegment::CircularArc {
                                start_mm: right,
                                end_mm: left,
                                center_mm: center,
                                clockwise: false,
                            },
                        ],
                        closed: true,
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(2),
                    name: "Circle extrusion".to_owned(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(3),
                        height: Dimension::new("20", 20.0).unwrap(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(2),
                    name: "Circle tool occurrence".to_owned(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        app.document.discard_history_before_current();
        app
    }

    fn solid_tool_graph(snapshot: &Snapshot, definition_id: DefinitionId) -> ExactBRepGraph {
        let producer_feature_id = *snapshot
            .definition(definition_id)
            .unwrap()
            .feature_ids()
            .last()
            .unwrap();
        ExactBRepGraph::from_snapshot(snapshot, definition_id, producer_feature_id).unwrap()
    }

    let target = || SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = || SelectionId {
        definition_id: DefinitionId(2),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };

    let mut subtract = app_with_circle_tool([40.0, 30.0], 10.0);
    subtract.active_tool = ActiveTool::SolidSubtract;
    let subtract_before_digest = subtract.canonical_digest();
    let subtract_before_revision = subtract.document_revision();
    let subtract_before_undo = subtract.document.visible_undo_steps();
    subtract.solid_tool_target = Some(target());
    assert!(subtract.prepare_solid_tool_preview(tool(), false));
    assert_eq!(subtract.canonical_digest(), subtract_before_digest);
    assert_eq!(subtract.document_revision(), subtract_before_revision);
    assert_eq!(subtract.document.visible_undo_steps(), subtract_before_undo);
    assert_eq!(
        subtract.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::ZERO, Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(
        subtract.push_pull_preview_exact_evaluator(),
        Some(ketchup_core::exact_product::EXACT_BREP_GRAPH_EVALUATOR_V1)
    );
    subtract.clear_ephemeral_edit_state();
    assert!(!subtract.has_occurrence_operation_preview());
    assert_eq!(subtract.canonical_digest(), subtract_before_digest);

    subtract.active_tool = ActiveTool::SolidSubtract;
    subtract.solid_tool_target = Some(target());
    assert!(subtract.prepare_solid_tool_preview(tool(), false));
    assert!(subtract.confirm_occurrence_operation_preview());
    assert_eq!(
        subtract.document.visible_undo_steps(),
        subtract_before_undo + 1
    );
    let subtract_committed = subtract.document.current();
    let subtract_definition = subtract_committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(subtract_committed.occurrence(OccurrenceId(2)).is_none());
    let subtract_graph = solid_tool_graph(&subtract_committed, subtract_definition);
    assert!(subtract_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Cut,
            ..
        }
    )));
    let subtract_digest = subtract_committed.canonical_digest();
    let subtract_reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&subtract_committed))
            .unwrap()
            .snapshot();
    assert_eq!(subtract_reopened.canonical_digest(), subtract_digest);
    assert_eq!(
        solid_tool_graph(&subtract_reopened, subtract_definition),
        subtract_graph
    );
    assert!(subtract.undo());
    assert_eq!(subtract.canonical_digest(), subtract_before_digest);
    assert!(subtract.redo());
    assert_eq!(subtract.canonical_digest(), subtract_digest);

    for (center, radius) in [([10.0, 30.0], 10.0), ([120.0, 30.0], 5.0)] {
        let mut rejected = app_with_circle_tool(center, radius);
        rejected.active_tool = ActiveTool::SolidSubtract;
        let digest = rejected.canonical_digest();
        rejected.solid_tool_target = Some(target());
        assert!(rejected.prepare_solid_tool_preview(tool(), false));
        assert_eq!(rejected.canonical_digest(), digest);
        assert!(rejected.has_occurrence_operation_preview());
    }

    let mut union = app_with_circle_tool([50.0, 30.0], 70.0);
    union.active_tool = ActiveTool::SolidUnion;
    let union_before_digest = union.canonical_digest();
    let union_before_revision = union.document_revision();
    let union_before_undo = union.document.visible_undo_steps();
    union.solid_tool_target = Some(target());
    assert!(union.prepare_solid_tool_preview(tool(), false));
    assert_eq!(union.canonical_digest(), union_before_digest);
    assert_eq!(union.document_revision(), union_before_revision);
    assert_eq!(union.document.visible_undo_steps(), union_before_undo);
    assert_eq!(
        union.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(-20.0, -40.0, 0.0), Vec3::new(140.0, 140.0, 20.0)))
    );
    union.clear_ephemeral_edit_state();
    assert!(!union.has_occurrence_operation_preview());
    assert_eq!(union.canonical_digest(), union_before_digest);

    union.active_tool = ActiveTool::SolidUnion;
    union.solid_tool_target = Some(target());
    assert!(union.prepare_solid_tool_preview(tool(), false));
    assert!(union.confirm_occurrence_operation_preview());
    assert_eq!(union.document.visible_undo_steps(), union_before_undo + 1);
    let union_committed = union.document.current();
    let union_definition = union_committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(union_committed.occurrence(OccurrenceId(2)).is_none());
    let union_graph = solid_tool_graph(&union_committed, union_definition);
    assert!(union_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    let union_digest = union_committed.canonical_digest();
    let union_reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&union_committed))
            .unwrap()
            .snapshot();
    assert_eq!(union_reopened.canonical_digest(), union_digest);
    assert_eq!(
        solid_tool_graph(&union_reopened, union_definition),
        union_graph
    );
    assert!(union.undo());
    assert_eq!(union.canonical_digest(), union_before_digest);
    assert!(union.redo());
    assert_eq!(union.canonical_digest(), union_digest);

    for (center, radius) in [
        ([40.0, 30.0], 10.0),
        ([50.0, 30.0], 55.0),
        ([150.0, 30.0], 10.0),
        ([50.0, 30.0], 58.309_518_948_453_004),
    ] {
        let mut rejected = app_with_circle_tool(center, radius);
        rejected.active_tool = ActiveTool::SolidUnion;
        let digest = rejected.canonical_digest();
        rejected.solid_tool_target = Some(target());
        assert!(rejected.prepare_solid_tool_preview(tool(), false));
        assert_eq!(rejected.canonical_digest(), digest);
        assert!(rejected.has_occurrence_operation_preview());
    }

    let mut app = app_with_circle_tool([40.0, 30.0], 10.0);
    app.active_tool = ActiveTool::SolidIntersect;
    let before_digest = app.canonical_digest();
    let before_revision = app.document_revision();
    let before_undo = app.document.visible_undo_steps();
    app.solid_tool_target = Some(target());
    assert!(app.prepare_solid_tool_preview(tool(), false));
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.document.visible_undo_steps(), before_undo);
    assert_eq!(
        app.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(30.0, 20.0, 0.0), Vec3::new(20.0, 20.0, 20.0)))
    );
    app.clear_ephemeral_edit_state();
    assert!(!app.has_occurrence_operation_preview());
    assert_eq!(app.canonical_digest(), before_digest);

    app.active_tool = ActiveTool::SolidIntersect;
    app.solid_tool_target = Some(target());
    assert!(app.prepare_solid_tool_preview(tool(), false));
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.document.visible_undo_steps(), before_undo + 1);
    let committed = app.document.current();
    let result_definition = committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(committed.occurrence(OccurrenceId(2)).is_none());
    let result_graph = solid_tool_graph(&committed, result_definition);
    assert!(result_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
    let committed_digest = committed.canonical_digest();
    let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&committed))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert_eq!(solid_tool_graph(&reopened, result_definition), result_graph);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed_digest);

    let mut split = app_with_circle_tool([40.0, 30.0], 10.0);
    split.active_tool = ActiveTool::SolidSplit;
    let split_before_digest = split.canonical_digest();
    let split_before_revision = split.document_revision();
    let split_before_undo = split.document.visible_undo_steps();
    split.solid_tool_target = Some(target());
    assert!(split.prepare_solid_tool_preview(tool(), false));
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert_eq!(split.document_revision(), split_before_revision);
    assert_eq!(split.document.visible_undo_steps(), split_before_undo);
    assert_eq!(
        split.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    split.clear_ephemeral_edit_state();
    assert!(!split.has_occurrence_operation_preview());
    assert_eq!(split.canonical_digest(), split_before_digest);

    split.active_tool = ActiveTool::SolidSplit;
    split.solid_tool_target = Some(target());
    assert!(split.prepare_solid_tool_preview(tool(), false));
    assert!(split.confirm_occurrence_operation_preview());
    assert_eq!(split.document.visible_undo_steps(), split_before_undo + 1);
    let split_committed = split.document.current();
    let split_definition = split_committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    assert!(split_committed.occurrence(OccurrenceId(2)).is_some());
    let split_graph = solid_tool_graph(&split_committed, split_definition);
    assert!(split_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Split,
            ..
        }
    )));
    let split_digest = split_committed.canonical_digest();
    let split_reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&split_committed))
            .unwrap()
            .snapshot();
    assert_eq!(split_reopened.canonical_digest(), split_digest);
    assert_eq!(
        solid_tool_graph(&split_reopened, split_definition),
        split_graph
    );
    assert!(split.undo());
    assert_eq!(split.canonical_digest(), split_before_digest);
    assert!(split.redo());
    assert_eq!(split.canonical_digest(), split_digest);

    for active_tool in [ActiveTool::SolidIntersect, ActiveTool::SolidSplit] {
        let mut boundary_tangent = app_with_circle_tool([10.0, 30.0], 10.0);
        boundary_tangent.active_tool = active_tool;
        let digest = boundary_tangent.canonical_digest();
        boundary_tangent.solid_tool_target = Some(target());
        assert!(boundary_tangent.prepare_solid_tool_preview(tool(), false));
        assert_eq!(boundary_tangent.canonical_digest(), digest);
        assert!(boundary_tangent.has_occurrence_operation_preview());

        let mut disjoint = app_with_circle_tool([120.0, 30.0], 5.0);
        disjoint.active_tool = active_tool;
        let digest = disjoint.canonical_digest();
        disjoint.solid_tool_target = Some(target());
        assert!(!disjoint.prepare_solid_tool_preview(tool(), false));
        assert_eq!(disjoint.canonical_digest(), digest);
        assert!(!disjoint.has_occurrence_operation_preview());
    }
}

#[test]
fn imported_exact_occurrences_route_through_solid_tool_preview_and_commit() {
    let mut app = KetchupApp::new();
    app.document = DocumentStore::new();
    let sources = [
        b"target imported exact body".as_slice(),
        b"tool imported exact body".as_slice(),
    ];
    let evidences = [
        StepImportEvidence {
            source_unit: ImportLengthUnit::Millimetre,
            result_fingerprint: "target-exact-result".into(),
            solid_count: 1,
            topology_counts: [8, 12, 6, 1, 1],
            volume_mm3: 1_000.0,
            bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
            backend: "headless-imported-solid-tool.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
        StepImportEvidence {
            source_unit: ImportLengthUnit::Millimetre,
            result_fingerprint: "tool-exact-result".into(),
            solid_count: 1,
            topology_counts: [8, 12, 6, 1, 1],
            volume_mm3: 432.0,
            bounds_mm: [[0.0, 0.0, 0.0], [6.0, 6.0, 12.0]],
            backend: "headless-imported-solid-tool.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
    ];
    for (index, (source, evidence)) in sources.iter().zip(&evidences).enumerate() {
        app.document
            .apply_batch(
                &plan_step_import(
                    &app.document.current(),
                    source,
                    &format!("imported-{index}.step"),
                    evidence,
                )
                .unwrap(),
            )
            .unwrap();
    }
    app.document.discard_history_before_current();
    let snapshot = app.document.current();
    let mut occurrences = snapshot.occurrences().map(|occurrence| occurrence.id());
    let target_occurrence_id = occurrences.next().unwrap();
    let tool_occurrence_id = occurrences.next().unwrap();
    let target_definition_id = snapshot
        .occurrence(target_occurrence_id)
        .unwrap()
        .definition_id();
    let tool_definition_id = snapshot
        .occurrence(tool_occurrence_id)
        .unwrap()
        .definition_id();
    let target_feature_id = exact_solid_tool_feature_id(&snapshot, target_definition_id).unwrap();
    let tool_feature_id = exact_solid_tool_feature_id(&snapshot, tool_definition_id).unwrap();
    let angle = 30.0_f64.to_radians();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: target_occurrence_id,
                transform: Transform::from_translation(2.0, -1.0, 0.5).unwrap(),
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: tool_occurrence_id,
                transform: Transform::from_matrix([
                    angle.cos(),
                    -angle.sin(),
                    0.0,
                    4.0,
                    angle.sin(),
                    angle.cos(),
                    0.0,
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    2.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                ])
                .unwrap(),
            },
        ]))
        .unwrap();
    app.document.discard_history_before_current();

    let selection = |definition_id, occurrence_id| SelectionId {
        definition_id,
        instance_path: InstancePath::root(occurrence_id),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.active_tool = ActiveTool::SolidIntersect;
    let imported_boxes = app.active_boxes();
    assert_eq!(imported_boxes.len(), 2, "{imported_boxes:?}");
    assert!(app.command_enabled(AppCommand::SolidIntersect));
    let before_digest = app.canonical_digest();
    let before_revision = app.document_revision();
    let before_undo_steps = app.undo_step_count();
    app.solid_tool_target = Some(selection(target_definition_id, target_occurrence_id));
    assert!(
        app.prepare_solid_tool_preview(selection(tool_definition_id, tool_occurrence_id), true,)
    );
    assert!(app.has_occurrence_operation_preview());
    let source = &app
        .occurrence_operation_preview
        .as_ref()
        .unwrap()
        .solid_tool_plan
        .as_ref()
        .unwrap()
        .source;
    assert_eq!(source.target_feature_id, target_feature_id);
    assert_eq!(source.tool_feature_id, tool_feature_id);
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.undo_step_count(), before_undo_steps);

    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.undo_step_count(), before_undo_steps + 1);
    let committed = app.document.current();
    let result_definition_id = committed
        .occurrence(target_occurrence_id)
        .unwrap()
        .definition_id();
    let result_feature_id = *committed
        .definition(result_definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&committed, result_definition_id, result_feature_id).unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::ImportedExact { .. }))
            .count(),
        2
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::RigidTransform { .. }))
            .count(),
        2
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
}

#[test]
fn mixed_extrusion_and_imported_exact_occurrences_route_through_solid_tools() {
    let mut app = KetchupApp::new();
    let source = b"mixed imported exact body";
    let evidence = StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "mixed-imported-exact-result".into(),
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        volume_mm3: 12_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [20.0, 30.0, 20.0]],
        backend: "headless-mixed-solid-tool.v1".into(),
        tolerance: "1e-7-mm".into(),
    };
    app.document
        .apply_batch(
            &plan_step_import(
                &app.document.current(),
                source,
                "mixed-imported.step",
                &evidence,
            )
            .unwrap(),
        )
        .unwrap();
    let snapshot = app.document.current();
    let imported_occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.id() != OccurrenceId(1))
        .unwrap();
    let imported_occurrence_id = imported_occurrence.id();
    let imported_definition_id = imported_occurrence.definition_id();
    app.container_data
        .insert_import_blob(source.to_vec())
        .unwrap();
    let target_angle = 15.0_f64.to_radians();
    let target_transform = Transform::from_matrix([
        target_angle.cos(),
        -target_angle.sin(),
        0.0,
        5.0,
        target_angle.sin(),
        target_angle.cos(),
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
    let tool_angle = 30.0_f64.to_radians();
    let tool_transform = Transform::from_matrix([
        tool_angle.cos(),
        -tool_angle.sin(),
        0.0,
        40.0,
        tool_angle.sin(),
        tool_angle.cos(),
        0.0,
        15.0,
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: target_transform,
            },
            CanonicalCommand::SetOccurrenceTransform {
                id: imported_occurrence_id,
                transform: tool_transform,
            },
        ]))
        .unwrap();
    app.document.discard_history_before_current();

    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = SelectionId {
        definition_id: imported_definition_id,
        instance_path: InstancePath::root(imported_occurrence_id),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    assert!(app.command_enabled(AppCommand::SolidSubtract));
    assert!(app.command_enabled(AppCommand::SolidUnion));
    assert!(app.command_enabled(AppCommand::SolidIntersect));
    app.active_tool = ActiveTool::SolidUnion;
    let before_digest = app.canonical_digest();
    let before_undo = app.undo_step_count();
    app.solid_tool_target = Some(target);
    assert!(app.prepare_solid_tool_preview(tool.clone(), true));
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.undo_step_count(), before_undo + 1);

    let committed = app.document.current();
    let result_definition_id = committed
        .occurrence(OccurrenceId(1))
        .unwrap()
        .definition_id();
    let result_feature_id = *committed
        .definition(result_definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&committed, result_definition_id, result_feature_id).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::Extrude { .. }))
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::ImportedExact { .. }))
    );
    let expected_relative_matrix = target_transform
        .rigid_inverse()
        .unwrap()
        .compose(tool_transform)
        .matrix()
        .map(f64::to_bits);
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if matrix_bits == expected_relative_matrix
    )));
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    let reopened = ketchup_core::persistence::load(
        &ketchup_core::persistence::save_container(&committed, &app.container_data).unwrap(),
    )
    .unwrap()
    .snapshot();
    assert_eq!(reopened.canonical_digest(), committed.canonical_digest());

    assert_eq!(
        exact_solid_tool_feature_id(&committed, result_definition_id),
        Some(result_feature_id)
    );
    assert!(
        !app.active_boxes()
            .iter()
            .any(|item| item.definition_id == result_definition_id),
        "graph-derived bodies without a current exact package must fail closed"
    );
    app.active_tool = ActiveTool::SolidSubtract;
    app.solid_tool_target = Some(SelectionId {
        definition_id: result_definition_id,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(!app.prepare_solid_tool_preview(tool.clone(), true));
    assert_eq!(app.canonical_digest(), committed.canonical_digest());

    install_graph_result(
        &mut app,
        result_definition_id,
        result_feature_id,
        Some([[-100.0, -100.0, -100.0], [200.0, 200.0, 200.0]]),
    );
    assert!(
        app.active_boxes()
            .iter()
            .any(|item| item.definition_id == result_definition_id),
        "a graph-derived body with a current exact package must be selectable"
    );
    assert!(app.prepare_solid_tool_preview(tool.clone(), true));
    let graph_preview = app
        .occurrence_operation_preview
        .as_ref()
        .unwrap()
        .solid_tool_plan
        .as_ref()
        .unwrap();
    assert_eq!(graph_preview.source.result_feature_ids.len(), 9);
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
    let graph_before_undo = app.undo_step_count();
    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.undo_step_count(), graph_before_undo + 1);
    let nested = app.document.current();
    let nested_definition_id = nested.occurrence(OccurrenceId(1)).unwrap().definition_id();
    let nested_definition = nested.definition(nested_definition_id).unwrap();
    assert_eq!(nested_definition.feature_ids().len(), 9);
    let nested_result = *nested_definition.feature_ids().last().unwrap();
    let nested_graph =
        ExactBRepGraph::from_snapshot(&nested, nested_definition_id, nested_result).unwrap();
    assert_eq!(
        nested_graph
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::Boolean { .. }))
            .count(),
        2
    );
    assert!(matches!(
        nested_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Cut,
            ..
        }
    ));
    let nested_digest = nested.canonical_digest();
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), nested_digest);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());

    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed.canonical_digest());
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_digest);

    app.active_tool = ActiveTool::SolidIntersect;
    app.solid_tool_target = Some(SelectionId {
        definition_id: imported_definition_id,
        instance_path: InstancePath::root(imported_occurrence_id),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    assert!(app.prepare_solid_tool_preview(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    ));
    let reverse_preview = app
        .occurrence_operation_preview
        .as_ref()
        .unwrap()
        .solid_tool_plan
        .as_ref()
        .unwrap();
    assert_eq!(
        reverse_preview.preview_box.profile_feature_id,
        reverse_preview.source.result_feature_ids[4]
    );
    assert!(reverse_preview.preview_box.extrusion_feature_id.is_none());
    assert!(app.confirm_occurrence_operation_preview());
    let reverse = app.document.current();
    assert!(reverse.occurrence(OccurrenceId(1)).is_none());
    let reverse_definition_id = reverse
        .occurrence(imported_occurrence_id)
        .unwrap()
        .definition_id();
    let reverse_feature_id = *reverse
        .definition(reverse_definition_id)
        .unwrap()
        .feature_ids()
        .last()
        .unwrap();
    let reverse_graph =
        ExactBRepGraph::from_snapshot(&reverse, reverse_definition_id, reverse_feature_id).unwrap();
    let reverse_relative_matrix = tool_transform
        .rigid_inverse()
        .unwrap()
        .compose(target_transform)
        .matrix()
        .map(f64::to_bits);
    assert!(reverse_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::RigidTransform { matrix_bits, .. }
            if matrix_bits == reverse_relative_matrix
    )));
    assert!(reverse_graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ketchup_core::exact_brep_graph::ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));
}

#[test]
fn solid_tool_preview_survives_accepted_exact_bounds_refresh_and_commits_once() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let tool = SelectionId {
        definition_id: app
            .document
            .current()
            .occurrence(OccurrenceId(2))
            .unwrap()
            .definition_id(),
        instance_path: InstancePath::root(OccurrenceId(2)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.active_tool = ActiveTool::SolidIntersect;
    app.solid_tool_target = Some(target);
    assert!(app.prepare_solid_tool_preview(tool, false));
    let preview_geometry = app
        .occurrence_operation_preview_geometry(OccurrenceId(1))
        .unwrap();
    let snapshot = app.document.current();
    let source_document_id = snapshot.document_id();
    let before_revision = snapshot.revision_id();
    let before_digest = snapshot.canonical_digest();
    let before_undo_steps = app.undo_step_count();

    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION, FeatureId(2)).unwrap();
    let minimum = Vec3::new(-10.0, -5.0, 0.0);
    let maximum = Vec3::new(110.0, 75.0, 20.0);
    let size = maximum - minimum;
    let vertices_mm = box_corners(size.x, size.y, size.z)
        .map(|point| {
            let point = point + minimum;
            [point.x, point.y, point.z]
        })
        .to_vec();
    let triangles = [
        ([0, 2, 1], 0),
        ([1, 2, 3], 0),
        ([4, 5, 6], 1),
        ([5, 7, 6], 1),
        ([0, 1, 4], 2),
        ([1, 5, 4], 2),
        ([2, 6, 3], 3),
        ([3, 6, 7], 3),
        ([0, 4, 2], 4),
        ([2, 4, 6], 4),
        ([1, 3, 5], 5),
        ([3, 7, 5], 5),
    ]
    .into_iter()
    .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
        vertex_indices,
        face_ordinal,
    })
    .collect();
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            exact_input_digest: "solid-tool-refreshed-input".into(),
            result_fingerprint: "solid-tool-refreshed-result".into(),
            volume_mm3: size.x * size.y * size.z,
            topology_counts: [8, 12, 6, 1, 1],
            bounds_mm: [
                [minimum.x, minimum.y, minimum.z],
                [maximum.x, maximum.y, maximum.z],
            ],
            backend: "solid-tool-headless-backend.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
        &StepImportMesh {
            vertices_mm,
            triangles,
        },
    )
    .unwrap();
    assert!(app.headless_install_exact_package(ExactBodyPackage::Graph(package)));

    assert_eq!(app.document.current().document_id(), source_document_id);
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.canonical_digest(), before_digest);
    assert_eq!(app.undo_step_count(), before_undo_steps);
    assert_eq!(
        app.occurrence_box_geometry(1),
        Some((minimum, size)),
        "the accepted exact package must change only the target's derived bounds"
    );
    assert_eq!(
        app.occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some(preview_geometry)
    );
    assert!(app.has_occurrence_operation_preview());

    assert!(app.confirm_occurrence_operation_preview());
    assert_eq!(app.document_revision(), before_revision + 1);
    assert_eq!(app.undo_step_count(), before_undo_steps + 1);
}

#[test]
fn solid_tool_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_intersection() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_box());
        let target = SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        let tool = SelectionId {
            definition_id: app
                .document
                .current()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .definition_id(),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        app.active_tool = ActiveTool::SolidIntersect;
        app.solid_tool_target = Some(target);
        assert!(app.prepare_solid_tool_preview(tool, false));
        assert!(app.has_occurrence_operation_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut command_tamper = prepared_intersection();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .solid_tool_plan
        .as_mut()
        .unwrap()
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!command_tamper.has_occurrence_operation_preview());
    assert!(!command_tamper.confirm_occurrence_operation_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_intersection();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .solid_tool_plan
        .as_mut()
        .unwrap()
        .source
        .tool_transform = Transform::identity();
    assert!(!source_tamper.confirm_occurrence_operation_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_intersection();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_occurrence_operation_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_intersection();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_occurrence_operation_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_intersection();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(2),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_occurrence_operation_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_intersection();
    let before_digest = valid.canonical_digest();
    let before_revision = valid.document_revision();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_occurrence_operation_preview());
    let committed_digest = valid.canonical_digest();
    assert_eq!(valid.document_revision(), before_revision + 1);
    assert_eq!(valid.undo_step_count(), before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    let committed_revision = valid.document_revision();
    let committed_undo_steps = valid.undo_step_count();
    assert!(!valid.confirm_occurrence_operation_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn revolve_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_revolve() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_closed_polyline(vec![
            [120.0, 0.0],
            [140.0, 0.0],
            [140.0, 30.0],
            [120.0, 30.0],
        ]));
        assert!(app.begin_revolve_tool());
        assert!(!app.add_revolve_axis_point(Vec3::new(110.0, 0.0, 0.0)));
        assert!(app.add_revolve_axis_point(Vec3::new(110.0, 30.0, 0.0)));
        assert!(app.has_revolve_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_revolve();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.revolve_preview.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.has_revolve_preview());
    assert!(!batch_tamper.confirm_revolve_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_revolve();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper.revolve_preview.as_mut().unwrap().plan.command =
        CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        };
    assert!(!plan_tamper.confirm_revolve_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_revolve();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .revolve_preview
        .as_mut()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    };
    assert!(!source_tamper.confirm_revolve_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_revolve();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_revolve_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_revolve();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_revolve_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_revolve();
    let occurrence_id = stale
        .revolve_preview
        .as_ref()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_revolve_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_revolve();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_revolve_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_revolve_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn planar_offset_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_planar_offset() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_closed_polyline(vec![
            [120.0, 0.0],
            [220.0, 0.0],
            [220.0, 60.0],
            [120.0, 60.0],
        ]));
        app.dispatch_command(AppCommand::PlanarOffset);
        assert!(app.planar_offset_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_planar_offset();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.planar_offset_preview.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.planar_offset_preview_is_current());
    assert!(!batch_tamper.confirm_planar_offset_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_planar_offset();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper
        .planar_offset_preview
        .as_mut()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_planar_offset_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_planar_offset();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .planar_offset_preview
        .as_mut()
        .unwrap()
        .plan
        .exact_request
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_planar_offset_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_planar_offset();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .planar_offset_preview
        .as_mut()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    };
    assert!(!source_tamper.confirm_planar_offset_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_planar_offset();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_planar_offset_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_planar_offset();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_planar_offset_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_planar_offset();
    let occurrence_id = stale
        .planar_offset_preview
        .as_ref()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_planar_offset_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_planar_offset();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_planar_offset_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_planar_offset_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn sweep_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_sweep() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_sweep_inputs(
            vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
            [0.0, 0.0],
            [0.0, 125.0],
        ));
        app.dispatch_command(AppCommand::Sweep);
        assert!(app.sweep_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_sweep();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.sweep_preview.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.sweep_preview_is_current());
    assert!(!batch_tamper.confirm_sweep_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_sweep();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper.sweep_preview.as_mut().unwrap().plan.command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_sweep_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_sweep();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .sweep_preview
        .as_mut()
        .unwrap()
        .plan
        .exact_request
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_sweep_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_sweep();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .sweep_preview
        .as_mut()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    };
    assert!(!source_tamper.confirm_sweep_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_sweep();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_sweep_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_sweep();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_sweep_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_sweep();
    let occurrence_id = stale
        .sweep_preview
        .as_ref()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_sweep_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_sweep();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_sweep_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_sweep_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn loft_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_loft() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.create_loft_inputs(vec![
            (
                vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
                0.0,
            ),
            (
                vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
                80.0,
            ),
        ]));
        app.dispatch_command(AppCommand::Loft);
        assert!(app.loft_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_loft();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.loft_preview.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.loft_preview_is_current());
    assert!(!batch_tamper.confirm_loft_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut plan_tamper = prepared_loft();
    let revision = plan_tamper.document_revision();
    let digest = plan_tamper.canonical_digest();
    let undo_steps = plan_tamper.undo_step_count();
    plan_tamper.loft_preview.as_mut().unwrap().plan.command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!plan_tamper.confirm_loft_preview());
    assert_unchanged(&plan_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_loft();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .loft_preview
        .as_mut()
        .unwrap()
        .plan
        .exact_request
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_loft_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_loft();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .loft_preview
        .as_mut()
        .unwrap()
        .plan
        .source
        .profile_kinds[0]
        .1 = FeatureKind::SplineProfile {
        control_points_mm: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    };
    assert!(!source_tamper.confirm_loft_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_loft();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_loft_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_loft();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_loft_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_loft();
    let occurrence_id = stale
        .loft_preview
        .as_ref()
        .unwrap()
        .plan
        .source
        .source_primary
        .as_ref()
        .unwrap()
        .instance_path
        .root_occurrence();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: occurrence_id,
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_loft_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_loft();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_loft_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_loft_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn general_finish_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_shell() -> KetchupApp {
        let mut app = KetchupApp::new();
        install_initial_graph_result(&mut app);
        select_initial_topological(&mut app, TopologicalElementKind::Face, 3);
        app.dispatch_command(AppCommand::Shell);
        assert!(app.general_finish_preview_is_current());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_shell();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.general_finish_preview.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.general_finish_preview_is_current());
    assert!(!batch_tamper.confirm_general_finish_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_shell();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .general_finish_preview
        .as_mut()
        .unwrap()
        .plan
        .command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!command_tamper.confirm_general_finish_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_shell();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .general_finish_preview
        .as_mut()
        .unwrap()
        .plan
        .exact_graph
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_general_finish_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_shell();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .general_finish_preview
        .as_mut()
        .unwrap()
        .plan
        .source
        .target_feature_kind = FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
    };
    assert!(!source_tamper.confirm_general_finish_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut amount_tamper = prepared_shell();
    let revision = amount_tamper.document_revision();
    let digest = amount_tamper.canonical_digest();
    let undo_steps = amount_tamper.undo_step_count();
    amount_tamper
        .general_finish_preview
        .as_mut()
        .unwrap()
        .plan
        .amount_mm_bits = 3.0_f64.to_bits();
    assert!(!amount_tamper.confirm_general_finish_preview());
    assert_unchanged(&amount_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_shell();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_general_finish_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_shell();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_general_finish_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut stale = prepared_shell();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_general_finish_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_shell();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_general_finish_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_general_finish_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn cut_through_adds_a_bounded_profile_to_the_selected_solid_as_one_undo_step() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );

    app.dispatch_command(AppCommand::CutThrough);
    assert_eq!(app.active_tool, ActiveTool::CutThrough);
    app.sketch_start = Some(Vec3::new(20.0, 15.0, 20.0));
    app.sketch_cursor = Some(Vec3::new(21.0, 16.0, 20.0));
    app.value_input = "30,20".to_owned();
    assert!(app.apply_value_input());

    let snapshot = app.document.current();
    assert!(matches!(
        snapshot.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Profile { points_mm }
            if points_mm == &vec![[20.0, 15.0], [50.0, 15.0], [50.0, 35.0], [20.0, 35.0]]
    ));
    assert!(matches!(
        snapshot.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::ThroughCut {
            target: FeatureId(2),
            profile: FeatureId(3),
        }
    ));
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert!(ExactFeatureChainRequest::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION).is_ok());
    let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&snapshot))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    assert!(matches!(
        reopened.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::ThroughCut { .. }
    ));

    assert!(app.undo());
    assert!(app.document.current().feature(FeatureId(3)).is_none());
    assert!(app.document.current().feature(FeatureId(4)).is_none());
    assert!(app.redo());
    assert!(matches!(
        app.document.current().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::ThroughCut { .. }
    ));
}

#[test]
fn topology_bound_push_pull_uses_the_selected_planar_face_and_rejects_tamper() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    select_initial_topological(&mut app, TopologicalElementKind::Face, 3);
    assert_eq!(
        app.selected_reference().unwrap().element,
        ElementId::Face {
            axis: Axis::Y,
            side: Side::Maximum,
        }
    );
    let source_digest = app.canonical_digest();
    let source_revision = app.document_revision();
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    assert_eq!(app.document_revision(), source_revision);
    assert_eq!(app.canonical_digest(), source_digest);
    let preview = app.preview_box.as_ref().unwrap();
    assert_eq!(preview.plan.preview_box.size_mm.y, 65.0);
    assert_eq!(preview.plan.preview_box.size_mm.z, 20.0);
    let topology = preview.plan.source.topological_selection.as_ref().unwrap();
    assert_eq!(
        preview.plan.source.topological_reference.as_ref(),
        Some(&topology.target().reference)
    );
    let manual_batch = preview.batch.clone();
    let source = preview.plan.source.clone();
    let (_, assistant_batch, assistant_proposal) = app
        .derive_push_pull_preview_plan(&source, ProposalPrincipal::LocalAssistant, "5", 5.0)
        .unwrap();
    assert_eq!(assistant_batch, manual_batch);
    assert_eq!(
        assistant_proposal.principal(),
        ProposalPrincipal::LocalAssistant
    );
    assert!(app.confirm_preview());
    assert_eq!(
        app.active_boxes().into_iter().next().unwrap().size_mm.y,
        65.0
    );
    let committed_digest = app.canonical_digest();
    let reopened =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&app.document.current()))
            .unwrap()
            .snapshot();
    assert_eq!(reopened.canonical_digest(), committed_digest);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), source_digest);
    assert!(app.redo());
    assert_eq!(app.canonical_digest(), committed_digest);

    let mut minimum_face = KetchupApp::new();
    install_initial_graph_result(&mut minimum_face);
    select_initial_topological(&mut minimum_face, TopologicalElementKind::Face, 4);
    assert_eq!(
        minimum_face.selected_reference().unwrap().element,
        ElementId::Face {
            axis: Axis::X,
            side: Side::Minimum,
        }
    );
    minimum_face.set_push_pull_distance_input("5");
    assert!(minimum_face.start_preview());
    let box_preview = &minimum_face.preview_box.as_ref().unwrap().plan.preview_box;
    assert_eq!(box_preview.origin_mm.x, -5.0);
    assert_eq!(box_preview.size_mm.x, 105.0);

    let revision = minimum_face.document_revision();
    let digest = minimum_face.canonical_digest();
    let undo_steps = minimum_face.undo_step_count();
    minimum_face
        .preview_box
        .as_mut()
        .unwrap()
        .plan
        .source
        .topological_reference
        .as_mut()
        .unwrap()
        .producer_element_id
        .push_str("-tampered");
    assert!(!minimum_face.confirm_preview());
    assert_eq!(minimum_face.document_revision(), revision);
    assert_eq!(minimum_face.canonical_digest(), digest);
    assert_eq!(minimum_face.undo_step_count(), undo_steps);
}

#[test]
fn push_pull_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_push_pull() -> KetchupApp {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        app.set_push_pull_distance_input("5");
        assert!(app.start_preview());
        assert!(app.has_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_push_pull();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.preview_box.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.confirm_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_push_pull();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .preview_box
        .as_mut()
        .unwrap()
        .plan
        .commands
        .clear();
    assert!(!command_tamper.confirm_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_push_pull();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .preview_box
        .as_mut()
        .unwrap()
        .plan
        .exact_request
        .as_mut()
        .unwrap()
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut geometry_tamper = prepared_push_pull();
    let revision = geometry_tamper.document_revision();
    let digest = geometry_tamper.canonical_digest();
    let undo_steps = geometry_tamper.undo_step_count();
    geometry_tamper
        .preview_box
        .as_mut()
        .unwrap()
        .plan
        .preview_box
        .size_mm
        .z += 1.0;
    assert!(!geometry_tamper.confirm_preview());
    assert_unchanged(&geometry_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_push_pull();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_push_pull();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut input_drift = prepared_push_pull();
    let revision = input_drift.document_revision();
    let digest = input_drift.canonical_digest();
    let undo_steps = input_drift.undo_step_count();
    input_drift.push_pull_distance_input = "6".to_owned();
    assert!(!input_drift.confirm_preview());
    assert_unchanged(&input_drift, revision, &digest, undo_steps);

    let mut stale = prepared_push_pull();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_push_pull();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn smart_push_pull_chooser_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_chooser() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.complete_circle(Vec3::new(35.0, 25.0, 20.0), 10.0, Vec3::new(1.0, 0.0, 0.0),));
        app.set_push_pull_distance_input("10");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        app.active_tool = ActiveTool::PushPull;
        app.value_input = "-10".to_owned();
        assert!(app.apply_value_input());
        assert!(app.has_smart_push_pull_chooser());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
        assert!(!app.has_occurrence_operation_preview());
        assert!(!app.has_preview());
    };

    let mut targets_tamper = prepared_chooser();
    let revision = targets_tamper.document_revision();
    let digest = targets_tamper.canonical_digest();
    let undo_steps = targets_tamper.undo_step_count();
    targets_tamper
        .smart_push_pull_chooser
        .as_mut()
        .unwrap()
        .source
        .targets
        .clear();
    assert!(!targets_tamper.confirm_smart_push_pull_choice());
    assert_unchanged(&targets_tamper, revision, &digest, undo_steps);

    let mut geometry_tamper = prepared_chooser();
    let revision = geometry_tamper.document_revision();
    let digest = geometry_tamper.canonical_digest();
    let undo_steps = geometry_tamper.undo_step_count();
    geometry_tamper
        .smart_push_pull_chooser
        .as_mut()
        .unwrap()
        .source
        .tool_box
        .size_mm
        .x += 1.0;
    assert!(!geometry_tamper.confirm_smart_push_pull_choice());
    assert_unchanged(&geometry_tamper, revision, &digest, undo_steps);

    let mut planning_tamper = prepared_chooser();
    let revision = planning_tamper.document_revision();
    let digest = planning_tamper.canonical_digest();
    let undo_steps = planning_tamper.undo_step_count();
    planning_tamper
        .smart_push_pull_chooser
        .as_mut()
        .unwrap()
        .planning = SmartPushPullPlanning::Append;
    assert!(!planning_tamper.confirm_smart_push_pull_choice());
    assert_unchanged(&planning_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_chooser();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_smart_push_pull_choice());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut input_drift = prepared_chooser();
    let revision = input_drift.document_revision();
    let digest = input_drift.canonical_digest();
    let undo_steps = input_drift.undo_step_count();
    input_drift.push_pull_distance_input = "-9".to_owned();
    assert!(!input_drift.confirm_smart_push_pull_choice());
    assert_unchanged(&input_drift, revision, &digest, undo_steps);

    let mut invalid_choice = prepared_chooser();
    let revision = invalid_choice.document_revision();
    let digest = invalid_choice.canonical_digest();
    let undo_steps = invalid_choice.undo_step_count();
    invalid_choice
        .smart_push_pull_chooser
        .as_mut()
        .unwrap()
        .selected = SmartPushPullChoice::ProfileCut(OccurrenceId(u64::MAX));
    assert!(!invalid_choice.confirm_smart_push_pull_choice());
    assert_unchanged(&invalid_choice, revision, &digest, undo_steps);

    let mut stale = prepared_chooser();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_smart_push_pull_choice());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_chooser();
    let target_id = valid
        .smart_push_pull_chooser
        .as_ref()
        .unwrap()
        .source
        .targets[0]
        .instance_path
        .root_occurrence();
    valid.smart_push_pull_chooser.as_mut().unwrap().selected =
        SmartPushPullChoice::ProfileCut(target_id);
    let revision = valid.document_revision();
    let digest = valid.canonical_digest();
    let undo_steps = valid.undo_step_count();
    assert!(valid.confirm_smart_push_pull_choice());
    assert_eq!(valid.document_revision(), revision);
    assert_eq!(valid.canonical_digest(), digest);
    assert_eq!(valid.undo_step_count(), undo_steps);
    assert!(valid.has_occurrence_operation_preview());
    assert!(!valid.confirm_smart_push_pull_choice());
    assert!(valid.has_occurrence_operation_preview());
}

#[test]
fn smart_through_cut_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_through_cut() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.complete_circle(Vec3::new(35.0, 25.0, 20.0), 10.0, Vec3::new(1.0, 0.0, 0.0),));
        app.set_push_pull_distance_input("10");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        app.active_tool = ActiveTool::PushPull;
        app.value_input = "-20".to_owned();
        assert!(app.apply_value_input());
        let chooser = app.smart_push_pull_chooser.as_mut().unwrap();
        chooser.selected = SmartPushPullChoice::ProfileCut(OccurrenceId(1));
        assert!(app.confirm_smart_push_pull_choice());
        assert!(app.has_occurrence_operation_preview());
        assert!(
            app.occurrence_operation_preview
                .as_ref()
                .unwrap()
                .smart_through_cut_plan
                .is_some()
        );
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_through_cut();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.confirm_profile_cut_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_through_cut();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .smart_through_cut_plan
        .as_mut()
        .unwrap()
        .commands
        .clear();
    assert!(!command_tamper.confirm_profile_cut_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_through_cut();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .smart_through_cut_plan
        .as_mut()
        .unwrap()
        .exact_request
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_profile_cut_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut geometry_tamper = prepared_through_cut();
    let revision = geometry_tamper.document_revision();
    let digest = geometry_tamper.canonical_digest();
    let undo_steps = geometry_tamper.undo_step_count();
    geometry_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .smart_through_cut_plan
        .as_mut()
        .unwrap()
        .source
        .target_box
        .size_mm
        .z += 1.0;
    assert!(!geometry_tamper.confirm_profile_cut_preview());
    assert_unchanged(&geometry_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_through_cut();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_profile_cut_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut input_drift = prepared_through_cut();
    let revision = input_drift.document_revision();
    let digest = input_drift.canonical_digest();
    let undo_steps = input_drift.undo_step_count();
    input_drift.push_pull_distance_input = "-19".to_owned();
    assert!(!input_drift.confirm_profile_cut_preview());
    assert_unchanged(&input_drift, revision, &digest, undo_steps);

    let mut stale = prepared_through_cut();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_profile_cut_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_through_cut();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_profile_cut_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_profile_cut_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.undo_step_count(), committed_undo_steps - 1);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn smart_profile_pocket_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_profile_pocket() -> KetchupApp {
        let mut app = KetchupApp::new();
        assert!(app.complete_circle(Vec3::new(35.0, 25.0, 20.0), 10.0, Vec3::new(1.0, 0.0, 0.0),));
        app.set_push_pull_distance_input("10");
        assert!(app.start_preview());
        assert!(app.confirm_preview());
        app.active_tool = ActiveTool::PushPull;
        app.value_input = "-10".to_owned();
        assert!(app.apply_value_input());
        let chooser = app.smart_push_pull_chooser.as_mut().unwrap();
        chooser.selected = SmartPushPullChoice::ProfileCut(OccurrenceId(1));
        assert!(app.confirm_smart_push_pull_choice());
        assert!(app.has_occurrence_operation_preview());
        let preview = app.occurrence_operation_preview.as_ref().unwrap();
        assert!(preview.smart_through_cut_plan.is_none());
        assert!(preview.smart_profile_pocket_plan.is_some());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_profile_pocket();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .batch = CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    }]);
    assert!(!batch_tamper.confirm_profile_cut_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_profile_pocket();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .smart_profile_pocket_plan
        .as_mut()
        .unwrap()
        .commands
        .clear();
    assert!(!command_tamper.confirm_profile_cut_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_profile_pocket();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .smart_profile_pocket_plan
        .as_mut()
        .unwrap()
        .exact_request
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_profile_cut_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut geometry_tamper = prepared_profile_pocket();
    let revision = geometry_tamper.document_revision();
    let digest = geometry_tamper.canonical_digest();
    let undo_steps = geometry_tamper.undo_step_count();
    geometry_tamper
        .occurrence_operation_preview
        .as_mut()
        .unwrap()
        .smart_profile_pocket_plan
        .as_mut()
        .unwrap()
        .translated_segments
        .clear();
    assert!(!geometry_tamper.confirm_profile_cut_preview());
    assert_unchanged(&geometry_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_profile_pocket();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_profile_cut_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_profile_pocket();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_profile_cut_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut input_drift = prepared_profile_pocket();
    let revision = input_drift.document_revision();
    let digest = input_drift.canonical_digest();
    let undo_steps = input_drift.undo_step_count();
    input_drift.push_pull_distance_input = "-9".to_owned();
    assert!(!input_drift.confirm_profile_cut_preview());
    assert_unchanged(&input_drift, revision, &digest, undo_steps);

    let mut stale = prepared_profile_pocket();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_profile_cut_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_profile_pocket();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_profile_cut_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_profile_cut_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.undo_step_count(), committed_undo_steps - 1);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn pocket_exact_plan_rejects_tamper_drift_stale_and_replay_atomically() {
    fn prepared_pocket() -> KetchupApp {
        let mut app = KetchupApp::new();
        select_initial_top_face(&mut app);
        app.dispatch_command(AppCommand::Pocket);
        assert!(app.prepare_pocket_preview(
            Vec3::new(20.0, 15.0, 20.0),
            Vec3::new(50.0, 35.0, 20.0),
            8.0,
        ));
        assert!(app.has_pocket_preview());
        app
    }

    let assert_unchanged = |app: &KetchupApp, revision, digest: &str, undo_steps| {
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    };

    let mut batch_tamper = prepared_pocket();
    let revision = batch_tamper.document_revision();
    let digest = batch_tamper.canonical_digest();
    let undo_steps = batch_tamper.undo_step_count();
    batch_tamper.pocket_preview.as_mut().unwrap().batch =
        CommandBatch::new(vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(1),
        }]);
    assert!(!batch_tamper.confirm_pocket_preview());
    assert_unchanged(&batch_tamper, revision, &digest, undo_steps);

    let mut command_tamper = prepared_pocket();
    let revision = command_tamper.document_revision();
    let digest = command_tamper.canonical_digest();
    let undo_steps = command_tamper.undo_step_count();
    command_tamper
        .pocket_preview
        .as_mut()
        .unwrap()
        .plan
        .profile_command = CanonicalCommand::DeleteOccurrence {
        id: OccurrenceId(1),
    };
    assert!(!command_tamper.confirm_pocket_preview());
    assert_unchanged(&command_tamper, revision, &digest, undo_steps);

    let mut request_tamper = prepared_pocket();
    let revision = request_tamper.document_revision();
    let digest = request_tamper.canonical_digest();
    let undo_steps = request_tamper.undo_step_count();
    request_tamper
        .pocket_preview
        .as_mut()
        .unwrap()
        .plan
        .exact_request
        .canonical_input_digest = "tampered".to_owned();
    assert!(!request_tamper.confirm_pocket_preview());
    assert_unchanged(&request_tamper, revision, &digest, undo_steps);

    let mut source_tamper = prepared_pocket();
    let revision = source_tamper.document_revision();
    let digest = source_tamper.canonical_digest();
    let undo_steps = source_tamper.undo_step_count();
    source_tamper
        .pocket_preview
        .as_mut()
        .unwrap()
        .plan
        .source
        .profile_kind = FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
    };
    assert!(!source_tamper.confirm_pocket_preview());
    assert_unchanged(&source_tamper, revision, &digest, undo_steps);

    let mut geometry_tamper = prepared_pocket();
    let revision = geometry_tamper.document_revision();
    let digest = geometry_tamper.canonical_digest();
    let undo_steps = geometry_tamper.undo_step_count();
    geometry_tamper
        .pocket_preview
        .as_mut()
        .unwrap()
        .plan
        .minimum_mm[0] += 1.0;
    assert!(!geometry_tamper.confirm_pocket_preview());
    assert_unchanged(&geometry_tamper, revision, &digest, undo_steps);

    let mut selection_drift = prepared_pocket();
    let revision = selection_drift.document_revision();
    let digest = selection_drift.canonical_digest();
    let undo_steps = selection_drift.undo_step_count();
    selection_drift.selection.clear();
    assert!(!selection_drift.confirm_pocket_preview());
    assert_unchanged(&selection_drift, revision, &digest, undo_steps);

    let mut context_drift = prepared_pocket();
    let revision = context_drift.document_revision();
    let digest = context_drift.canonical_digest();
    let undo_steps = context_drift.undo_step_count();
    context_drift
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!context_drift.confirm_pocket_preview());
    assert_unchanged(&context_drift, revision, &digest, undo_steps);

    let mut input_drift = prepared_pocket();
    let revision = input_drift.document_revision();
    let digest = input_drift.canonical_digest();
    let undo_steps = input_drift.undo_step_count();
    input_drift.value_input = "9".to_owned();
    assert!(!input_drift.confirm_pocket_preview());
    assert_unchanged(&input_drift, revision, &digest, undo_steps);

    let mut stale = prepared_pocket();
    stale
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let revision = stale.document_revision();
    let digest = stale.canonical_digest();
    let undo_steps = stale.undo_step_count();
    assert!(!stale.confirm_pocket_preview());
    assert_unchanged(&stale, revision, &digest, undo_steps);

    let mut valid = prepared_pocket();
    let before_revision = valid.document_revision();
    let before_digest = valid.canonical_digest();
    let before_undo_steps = valid.undo_step_count();
    assert!(valid.confirm_pocket_preview());
    let committed_revision = valid.document_revision();
    let committed_digest = valid.canonical_digest();
    let committed_undo_steps = valid.undo_step_count();
    assert_eq!(committed_revision, before_revision + 1);
    assert_eq!(committed_undo_steps, before_undo_steps + 1);
    assert_ne!(committed_digest, before_digest);
    assert!(!valid.confirm_pocket_preview());
    assert_unchanged(
        &valid,
        committed_revision,
        &committed_digest,
        committed_undo_steps,
    );
    assert!(valid.undo());
    assert_eq!(valid.canonical_digest(), before_digest);
    assert!(valid.redo());
    assert_eq!(valid.canonical_digest(), committed_digest);
}

#[test]
fn pocket_previews_then_commits_and_edits_depth_as_canonical_undo_steps() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    let original_digest = app.canonical_digest();

    app.dispatch_command(AppCommand::Pocket);
    app.sketch_start = Some(Vec3::new(20.0, 15.0, 20.0));
    app.sketch_cursor = Some(Vec3::new(21.0, 16.0, 20.0));
    app.value_input = "30,20".to_owned();
    assert!(app.apply_value_input());
    assert!(app.has_pocket_preview());
    assert_eq!(app.canonical_digest(), original_digest);
    assert!(!app.can_undo());

    app.value_input = "8".to_owned();
    assert!(app.apply_value_input());
    assert!(!app.has_pocket_preview());
    let snapshot = app.document.current();
    assert!(matches!(
        snapshot.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::Profile { points_mm }
            if points_mm == &vec![[20.0, 15.0], [50.0, 15.0], [50.0, 35.0], [20.0, 35.0]]
    ));
    assert!(matches!(
        snapshot.feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Pocket {
            target: FeatureId(2),
            profile: FeatureId(3),
            depth,
        } if depth.millimetres() == 8.0
    ));
    assert_eq!(app.document.visible_undo_steps(), 1);
    let reopened = ketchup_core::persistence::load(&ketchup_core::persistence::save(&snapshot))
        .unwrap()
        .snapshot();
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());

    assert!(app.set_selected_pocket_depth(12.0));
    assert_eq!(app.document.visible_undo_steps(), 2);
    assert!(matches!(
        app.document.current().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Pocket { depth, .. } if depth.millimetres() == 12.0
    ));
    assert!(app.undo());
    assert!(matches!(
        app.document.current().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Pocket { depth, .. } if depth.millimetres() == 8.0
    ));
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), original_digest);
    assert!(app.redo());
    assert!(matches!(
        app.document.current().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Pocket { depth, .. } if depth.millimetres() == 8.0
    ));
}

#[test]
fn pocket_preview_fails_closed_for_invalid_or_stale_depth() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.dispatch_command(AppCommand::Pocket);
    let digest = app.canonical_digest();
    assert!(!app.prepare_pocket_preview(
        Vec3::new(20.0, 15.0, 20.0),
        Vec3::new(50.0, 35.0, 20.0),
        20.0,
    ));
    assert_eq!(app.canonical_digest(), digest);

    assert!(app.prepare_pocket_preview(
        Vec3::new(20.0, 15.0, 20.0),
        Vec3::new(50.0, 35.0, 20.0),
        8.0,
    ));
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(10.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    assert!(!app.has_pocket_preview());
    assert!(!app.confirm_pocket_preview());
    assert!(app.document.current().feature(FeatureId(3)).is_none());
}

#[test]
fn cut_through_rejects_a_profile_that_touches_the_target_boundary() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.dispatch_command(AppCommand::CutThrough);
    let digest = app.canonical_digest();

    assert!(
        !app.complete_rectangle_sketch(Vec3::new(0.0, 15.0, 20.0), Vec3::new(50.0, 35.0, 20.0),)
    );
    assert_eq!(app.canonical_digest(), digest);
    assert!(!app.can_undo());
}

#[test]
fn cut_through_stays_disabled_for_an_exact_unsupported_offset_profile() {
    let mut app = KetchupApp::new();
    assert!(app.create_closed_polyline(vec![
        [10.0, 10.0],
        [80.0, 10.0],
        [80.0, 50.0],
        [10.0, 50.0],
    ]));
    app.set_push_pull_distance_input("20");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    let digest = app.canonical_digest();

    assert!(!app.command_enabled(AppCommand::CutThrough));
    app.dispatch_command(AppCommand::CutThrough);
    assert_ne!(app.active_tool, ActiveTool::CutThrough);
    assert_eq!(app.canonical_digest(), digest);
}

#[test]
fn closed_polyline_path_preserves_points_and_rejects_invalid_input_atomically() {
    let mut app = KetchupApp::new();
    let points_mm = vec![
        [-12.5, 4.25],
        [80.0, 4.25],
        [95.5, 40.0],
        [35.0, 72.75],
        [-12.5, 40.0],
    ];
    assert!(app.create_closed_polyline(points_mm.clone()));
    let created = app.active_boxes()[1].clone();
    assert_eq!(created.extrusion_feature_id, None);
    assert_eq!(created.size_mm.z, 0.0);
    assert!(matches!(
        app.document
            .current()
            .feature(created.profile_feature_id)
            .unwrap()
            .kind(),
        FeatureKind::Profile { points_mm: stored } if stored == &points_mm
    ));

    let digest = app.canonical_digest();
    let revision = app.document_revision();
    assert!(!app.create_closed_polyline(vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0],]));
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.document_revision(), revision);
}

#[test]
fn push_pull_keeps_the_opposite_face_fixed_on_screen() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let bottom = Vec3::new(0.0, 0.0, 0.0);
    let top = Vec3::new(0.0, 0.0, app.document_height_mm());
    let bottom_before = app.project(bottom, rect);
    let top_before = app.project(top, rect);

    app.set_push_pull_distance_input("20");
    assert!(app.start_preview());

    assert_eq!(app.project(bottom, rect), bottom_before);
    assert_ne!(app.project(Vec3::new(0.0, 0.0, 40.0), rect), top_before);
}

#[test]
fn confirmed_push_pull_can_be_undone_and_redone() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    assert!(!app.can_undo());

    app.set_push_pull_distance_input("22");
    assert!(app.start_preview());
    assert!(app.confirm_preview());
    assert_eq!(app.document_height_mm(), 42.0);

    assert!(app.undo());
    assert_eq!(app.document_height_mm(), 20.0);
    assert!(app.can_redo());

    assert!(app.redo());
    assert_eq!(app.document_height_mm(), 42.0);
}

#[test]
fn typed_push_pull_values_correct_the_last_one_instead_of_stacking() {
    let mut app = KetchupApp::new();
    let base_height = app.document_height_mm();
    app.active_tool = ActiveTool::PushPull;
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );

    let base_digest = app.canonical_digest();
    app.value_input = "20".to_owned();
    assert!(app.apply_value_input());
    let original_revision = app.document_revision();
    assert_eq!(app.document_height_mm(), base_height + 20.0);
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.value_input = "25".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document_revision(), original_revision + 1);
    assert_eq!(app.document_height_mm(), base_height + 25.0);
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.value_input = "0".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.document_revision(), original_revision + 2);
    assert_eq!(app.document_height_mm(), base_height);
    assert_eq!(app.document.visible_undo_steps(), 1);

    assert!(app.undo());
    assert_eq!(app.document_height_mm(), base_height);
    assert_eq!(app.canonical_digest(), base_digest);
    assert!(!app.can_undo(), "corrections must stay one undo step");
}

#[test]
fn rejected_push_pull_correction_preserves_the_last_valid_operation() {
    let mut app = KetchupApp::new();
    app.active_tool = ActiveTool::PushPull;
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.value_input = "20".to_owned();
    assert!(app.apply_value_input());
    let valid_height = app.document_height_mm();
    let valid_revision = app.document_revision();
    let valid_digest = app.canonical_digest();
    let valid_undo_steps = app.document.visible_undo_steps();

    app.value_input = "-100".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(app.document_height_mm(), valid_height);
    assert_eq!(app.document_revision(), valid_revision);
    assert_eq!(app.canonical_digest(), valid_digest);
    assert_eq!(app.document.visible_undo_steps(), valid_undo_steps);
    assert_eq!(
        app.last_push_pull
            .as_ref()
            .map(|operation| operation.canonical_digest.as_str()),
        Some(valid_digest.as_str())
    );
}

#[test]
fn rename_plans_are_revision_context_command_bound_and_clipboard_preserving() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();

    let occurrence_source = app.occurrence_rename_source_plan().unwrap();
    assert_eq!(occurrence_source.source_revision, app.document_revision());
    assert_eq!(occurrence_source.occurrence_count, 1);
    app.begin_occurrence_rename();
    app.pending_occurrence_rename.as_mut().unwrap().name = "Exact occurrence".to_owned();
    let occurrence_pending = app.pending_occurrence_rename.clone().unwrap();
    let occurrence_plan = app.occurrence_rename_plan(&occurrence_pending).unwrap();
    assert_eq!(
        occurrence_plan.command,
        CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "Exact occurrence".to_owned(),
        }
    );

    let mut tampered_occurrence_plan = occurrence_plan.clone();
    tampered_occurrence_plan.command = CanonicalCommand::RenameEntity {
        id: OccurrenceId(1),
        name: "Tampered occurrence".to_owned(),
    };
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    assert!(!app.apply_occurrence_rename_plan(tampered_occurrence_plan));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_occurrence_rename_plan(occurrence_plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
    app.selection.edit_context.clear();

    assert!(app.apply_occurrence_rename_plan(occurrence_plan));
    assert_eq!(
        app.occurrence_name(OccurrenceId(1)),
        Some("Exact occurrence".to_owned())
    );
    assert_eq!(app.occurrence_clipboard, clipboard);
    assert!(app.undo());

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.begin_definition_rename();
    app.pending_definition_rename.as_mut().unwrap().name = "Exact definition".to_owned();
    let definition_pending = app.pending_definition_rename.clone().unwrap();
    let definition_plan = app.definition_rename_plan(&definition_pending).unwrap();
    assert_eq!(definition_plan.source.instance_count, 1);
    assert_eq!(
        definition_plan.command,
        CanonicalCommand::RenameDefinition {
            id: INITIAL_BOX_DEFINITION,
            name: "Exact definition".to_owned(),
        }
    );

    let mut tampered_definition_plan = definition_plan.clone();
    tampered_definition_plan.source.instance_count += 1;
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    assert!(!app.apply_definition_rename_plan(tampered_definition_plan));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_definition_rename_plan(definition_plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
    app.selection.edit_context.clear();

    assert!(app.apply_definition_rename_plan(definition_plan));
    assert_eq!(
        app.definition_name(INITIAL_BOX_DEFINITION),
        Some("Exact definition".to_owned())
    );
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn viewport_selection_keeps_group_commands_available() {
    let mut app = KetchupApp::new();
    app.select_all();
    assert!(app.copy_selection_to_clipboard());
    assert!(app.paste_clipboard());
    app.select_all();
    assert!(app.group_selected());

    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.clear_selection();
    app.select_from_viewport(Some(target), false);

    assert_eq!(app.selection_count(), 2);
    assert!(app.selection.primary.is_none());
    assert!(app.selected_group_id().is_some());
    assert!(app.command_enabled(AppCommand::Ungroup));
    assert!(app.command_enabled(AppCommand::MakeComponent));
}

#[test]
fn group_and_ungroup_fail_closed_for_exact_incomplete_and_stale_selection() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection
        .select_path(InstancePath::root(OccurrenceId(1)), false);
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        true,
    );
    let exact_revision = app.document_revision();
    let exact_digest = app.canonical_digest();
    let exact_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::Group));
    assert!(!app.group_selected());
    assert_eq!(app.document_revision(), exact_revision);
    assert_eq!(app.canonical_digest(), exact_digest);
    assert_eq!(app.document.visible_undo_steps(), exact_undo_steps);

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    app.selection.primary = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    let face_revision = app.document_revision();
    let face_digest = app.canonical_digest();
    let face_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), face_revision);
    assert_eq!(app.canonical_digest(), face_digest);
    assert_eq!(app.document.visible_undo_steps(), face_undo_steps);

    app.selection.primary = None;
    app.selection
        .occurrences
        .remove(&InstancePath::root(OccurrenceId(2)));
    let incomplete_revision = app.document_revision();
    let incomplete_digest = app.canonical_digest();
    let incomplete_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::Ungroup));
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.ungroup_selected());
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), incomplete_revision);
    assert_eq!(app.canonical_digest(), incomplete_digest);
    assert_eq!(app.document.visible_undo_steps(), incomplete_undo_steps);

    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(2),
                parent: None,
            },
            CanonicalCommand::DeleteGroup { id: group_id },
        ]))
        .unwrap();
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::Ungroup));
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.ungroup_selected());
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.document.visible_undo_steps(), stale_undo_steps);
}

#[test]
fn group_and_ungroup_plans_are_revision_bound_exact_and_clipboard_preserving() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();

    let group_plan = app.group_selection_source_plan().unwrap();
    assert_eq!(group_plan.source_revision, app.document_revision());
    assert_eq!(group_plan.occurrence_count, 2);
    assert_eq!(group_plan.commands.len(), 3);
    let mut tampered_group_plan = group_plan.clone();
    tampered_group_plan.occurrence_count += 1;
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    assert!(!app.apply_group_selection_source_plan(tampered_group_plan));
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.create_box());
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_group_selection_source_plan(group_plan));
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let ungroup_plan = app.ungroup_selection_source_plan().unwrap();
    assert_eq!(ungroup_plan.source_revision, app.document_revision());
    assert_eq!(ungroup_plan.group_id, group_id);
    assert_eq!(ungroup_plan.occurrence_count, 2);
    assert_eq!(ungroup_plan.item_count, 2);

    let mut tampered_ungroup_plan = ungroup_plan.clone();
    tampered_ungroup_plan.commands.pop();
    let grouped_digest = app.canonical_digest();
    let grouped_undo_steps = app.undo_step_count();
    let grouped_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_ungroup_selection_source_plan(tampered_ungroup_plan));
    assert_eq!(app.canonical_digest(), grouped_digest);
    assert_eq!(app.undo_step_count(), grouped_undo_steps);
    assert_eq!(app.action_digest(), grouped_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(group_id));
    assert!(!app.apply_ungroup_selection_source_plan(ungroup_plan));
    assert_eq!(app.canonical_digest(), grouped_digest);
    assert_eq!(app.undo_step_count(), grouped_undo_steps);
    assert_eq!(app.action_digest(), grouped_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
    app.selection.edit_context.clear();

    assert!(app.ungroup_selected());
    assert_eq!(app.group_count(), 0);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn moved_group_behaves_as_one_object_and_explodes_without_geometry_shift() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_all();
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let ids = [OccurrenceId(1), OccurrenceId(2)];
    let before = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    let revision_before_move = app.document_revision();

    assert!(app.move_selected(Vec3::new(40.0, -20.0, 15.0)));
    assert_eq!(app.document_revision(), revision_before_move + 1);
    assert_eq!(app.selection.selected_group, Some(group_id));
    let moved = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    for (before, moved) in before.into_iter().zip(moved) {
        assert_eq!(moved.matrix()[3], before.matrix()[3] + 40.0);
        assert_eq!(moved.matrix()[7], before.matrix()[7] - 20.0);
        assert_eq!(moved.matrix()[11], before.matrix()[11] + 15.0);
    }

    let moved = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    assert!(app.ungroup_selected());
    assert_eq!(app.group_count(), 0);
    let exploded = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    assert_eq!(exploded, moved);
}

#[test]
fn zoom_selection_preserves_camera_basis_projection_and_document() {
    let mut app = KetchupApp::new();
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    assert!(!app.command_enabled(AppCommand::ZoomSelection));
    app.selection.select_occurrence(OccurrenceId(1), false);
    assert!(app.command_enabled(AppCommand::ZoomSelection));

    let basis = app.camera_basis();
    let projection = app.projection_mode();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    app.dispatch_command(AppCommand::ZoomSelection);

    assert_eq!(app.camera_basis(), basis);
    assert_eq!(app.projection_mode(), projection);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-zoom-selection",
            &BTreeMap::from([("count", "1".to_owned())]),
        )
    );
}

#[test]
fn zoom_steps_clamp_without_changing_camera_basis_projection_or_document() {
    let mut app = KetchupApp::new();
    let basis = app.camera_basis();
    let projection = app.projection_mode();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    app.zoom = MAX_CAMERA_ZOOM / 1.1;
    app.dispatch_command(AppCommand::ZoomIn);
    assert_eq!(app.camera_zoom(), MAX_CAMERA_ZOOM);
    assert!(!app.command_enabled(AppCommand::ZoomIn));

    app.zoom = MIN_CAMERA_ZOOM * 1.1;
    app.dispatch_command(AppCommand::ZoomOut);
    assert_eq!(app.camera_zoom(), MIN_CAMERA_ZOOM);
    assert!(!app.command_enabled(AppCommand::ZoomOut));

    assert_eq!(app.camera_basis(), basis);
    assert_eq!(app.projection_mode(), projection);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn standard_orthographic_views_match_the_drawing_axes_without_document_mutation() {
    fn assert_basis(actual: (Vec3, Vec3, Vec3), expected: (Vec3, Vec3, Vec3)) {
        for (actual, expected) in [
            (actual.0, expected.0),
            (actual.1, expected.1),
            (actual.2, expected.2),
        ] {
            assert!((actual.x - expected.x).abs() < 1.0e-6);
            assert!((actual.y - expected.y).abs() < 1.0e-6);
            assert!((actual.z - expected.z).abs() < 1.0e-6);
        }
    }

    let mut app = KetchupApp::new();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    for (command, key, basis) in [
        (
            AppCommand::ViewTop,
            "view-top",
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, -1.0),
            ),
        ),
        (
            AppCommand::ViewBottom,
            "view-bottom",
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
        ),
        (
            AppCommand::ViewFront,
            "view-front",
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
        ),
        (
            AppCommand::ViewBack,
            "view-back",
            (
                Vec3::new(-1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(0.0, -1.0, 0.0),
            ),
        ),
        (
            AppCommand::ViewRight,
            "view-right",
            (
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(-1.0, 0.0, 0.0),
            ),
        ),
        (
            AppCommand::ViewLeft,
            "view-left",
            (
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 0.0),
            ),
        ),
    ] {
        app.dispatch_command(command);
        assert_basis(app.camera_basis(), basis);
        assert_eq!(
            app.action_digest(),
            app.catalog.format(
                "digest-view-changed",
                &BTreeMap::from([("view", app.catalog.text(key))]),
            )
        );
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo_steps);
    }
}

#[test]
fn gpu_projection_matches_cpu_projection_inside_callback_viewport() {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::new(87.0, 163.0), Vec2::new(1_927.0, 1_184.0));

    for mode in [ProjectionMode::Perspective, ProjectionMode::Parallel] {
        app.projection_mode = mode;
        let matrix = app.world_to_clip(rect);
        for point in box_corners(BOX_WIDTH_MM, BOX_DEPTH_MM, app.document_height_mm()) {
            let clip_x = matrix[0] * point.x as f32
                + matrix[4] * point.y as f32
                + matrix[8] * point.z as f32
                + matrix[12];
            let clip_y = matrix[1] * point.x as f32
                + matrix[5] * point.y as f32
                + matrix[9] * point.z as f32
                + matrix[13];
            // The rasterizer divides by clip w before it maps to the
            // viewport, so the check has to divide as well or it would
            // only ever be valid for the parallel projection.
            let clip_w = matrix[3] * point.x as f32
                + matrix[7] * point.y as f32
                + matrix[11] * point.z as f32
                + matrix[15];
            let gpu_screen = Pos2::new(
                rect.center().x + (clip_x / clip_w) * rect.width() * 0.5,
                rect.center().y - (clip_y / clip_w) * rect.height() * 0.5,
            );
            let cpu_screen = app.project(point, rect);
            assert!((gpu_screen - cpu_screen).length() < 0.01, "{mode:?}");
        }
    }
}

#[test]
fn viewport_omits_edge_on_faces_that_collapse_to_a_line() {
    let mut app = KetchupApp::new();
    // Only a parallel projection collapses an edge-on face to a line; a
    // converging one always leaves a sliver of area.
    app.projection_mode = ProjectionMode::Parallel;
    app.yaw = std::f32::consts::FRAC_PI_2;
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let projected = box_corners(BOX_WIDTH_MM, BOX_DEPTH_MM, app.document_height_mm())
        .map(|point| app.project(point, rect));
    let forward = Vec3::new(
        -f64::from(app.yaw.sin() * app.pitch.sin()),
        -f64::from(app.yaw.cos() * app.pitch.sin()),
        -f64::from(app.pitch.cos()),
    );

    assert_eq!(
        box_faces()
            .into_iter()
            .filter(|face| {
                face_is_visible(&face.element, forward)
                    && projected_face_has_area(face.corners, &projected)
            })
            .count(),
        2
    );
}

#[test]
fn viewport_draws_only_the_three_camera_facing_box_faces() {
    let app = KetchupApp::new();
    let forward = Vec3::new(
        -f64::from(app.yaw.sin() * app.pitch.sin()),
        -f64::from(app.yaw.cos() * app.pitch.sin()),
        -f64::from(app.pitch.cos()),
    );

    assert_eq!(
        box_faces()
            .into_iter()
            .filter(|face| face_is_visible(&face.element, forward))
            .count(),
        3
    );
}

#[test]
fn viewport_click_routes_through_exact_spatial_query() {
    let app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let selected = app.exact_pick_at_screen(rect.center(), rect).unwrap();
    assert_eq!(selected.definition_id, INITIAL_BOX_DEFINITION);
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(1)));
    assert_eq!(
        selected.element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }
    );
}

#[test]
fn picking_chooses_the_frontmost_body_across_mesh_and_box_geometry() {
    let mut app = KetchupApp::new();
    assert!(apply_reviewed_model_intent(
        &mut app,
        AssistantModelIntent {
            replace_scene: true,
            boxes: vec![
                AssistantBoxIntent {
                    name: "Grooved behind".to_owned(),
                    size_mm: [100.0, 60.0, 20.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: vec![
                        ketchup_core::assistant_sidecar::AssistantSubtractionIntent {
                            size_mm: [10.0, 60.0, 5.0],
                            origin_mm: [45.0, 0.0, 15.0],
                        }
                    ],
                },
                AssistantBoxIntent {
                    name: "Plain in front".to_owned(),
                    size_mm: [100.0, 60.0, 20.0],
                    origin_mm: [0.0, 0.0, 40.0],
                    subtract_boxes: Vec::new(),
                },
            ],
            translations: Vec::new(),
            rotations: Vec::new(),
            profile_translations: Vec::new(),
            parameter_edits: Vec::new(),
            linear_arrays: Vec::new(),
            bottles: Vec::new(),
            gable_roofs: Vec::new(),
            staircases: Vec::new(),
            oriented_beams: Vec::new(),
            balloon_texts: Vec::new(),
        }
    ));
    app.projection_mode = ProjectionMode::Parallel;
    app.yaw = 0.0;
    app.pitch = 0.0;
    app.camera_target_z = 30.0;
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let pointer = app.project(Vec3::new(50.0, 30.0, 60.0), rect);

    let selected = app.exact_pick_at_screen(pointer, rect).unwrap();
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(3)));
}

#[test]
fn repeated_large_scene_picks_reuse_revision_bound_spatial_indices() {
    let mut app = KetchupApp::new();
    let source = app
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .clone();
    let commands = (2_u32..=480)
        .map(|id| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(u64::from(id)),
            definition_id: source.definition_id(),
            name: format!("Stacked {id}"),
            transform: Transform::from_translation(
                f64::from((id - 1) % 24) * 120.0,
                0.0,
                f64::from((id - 1) / 24) * 280.0,
            )
            .unwrap(),
            parent: None,
            tag: None,
            visible: true,
        })
        .collect();
    app.document
        .apply_batch(&CommandBatch::new(commands))
        .unwrap();
    app.zoom_fit();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
    let pointer = app.project(Vec3::new(50.0, 30.0, 20.0), rect);

    assert!(app.exact_pick_at_screen(pointer, rect).is_some());
    let cache_ptrs = app.interaction_projection_cache_ptrs().unwrap();
    for _ in 0..480 {
        assert!(app.exact_pick_at_screen(pointer, rect).is_some());
        assert_eq!(app.interaction_projection_cache_ptrs(), Some(cache_ptrs));
    }
}

#[test]
fn parallel_view_picks_a_hundred_metre_body_after_zoom_fit() {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
    app.viewport_rect = Some(rect);
    assert!(app.create_box_at(Vec3::new(0.0, 200.0, 0.0), Vec3::new(100_000.0, 60.0, 20.0),));
    app.zoom_fit();
    app.refresh_camera_distance();
    let visible_top = app.project(Vec3::new(75_000.0, 230.0, 20.0), rect);

    let selected = app
        .exact_pick_at_screen(visible_top, rect)
        .expect("a visible point on a 100 m body must remain pickable");

    assert_eq!(selected.definition_id, DefinitionId(2));
    assert_eq!(selected.instance_path, InstancePath::root(OccurrenceId(2)));
}

#[test]
fn viewport_picks_the_geometry_currently_shown_in_preview() {
    let mut app = KetchupApp::new();
    select_initial_top_face(&mut app);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    app.set_push_pull_distance_input("40");
    assert!(app.start_preview());
    let top_center = app.project(Vec3::new(50.0, 30.0, 60.0), rect);

    let selected = app.exact_pick_at_screen(top_center, rect).unwrap();

    assert_eq!(
        selected.element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }
    );
}

#[test]
fn dimensions_panel_creates_categories_and_assigns_independent_values_in_one_undo_step() {
    let mut app = KetchupApp::new();
    app.selection
        .select_path(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.create_classification_dimension("Building side", "Exterior"));
    assert!(app.add_classification_category(ClassificationDimensionId(1), "Interior"));
    assert!(app.create_classification_dimension("Building system", "Structure"));
    assert!(app.assign_selection_to_classification(
        ClassificationDimensionId(1),
        Some(ClassificationCategoryId(2))
    ));
    let undo_before = app.document.visible_undo_steps();
    assert!(app.assign_selection_to_classification(
        ClassificationDimensionId(2),
        Some(ClassificationCategoryId(3))
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(2))
    );
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(2)),
        Some(ClassificationCategoryId(3))
    );
    app.document.undo().unwrap();
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(2))
    );
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(2)),
        None
    );

    app.assistant_workspace_mode = AssistantWorkspaceMode::Tab;
    app.classification_selected_dimension = Some(ClassificationDimensionId(1));
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    for expected in ["DIMENSIONS", "Interior (1 occurrences)"] {
        assert!(
            harness
                .query_all_by(|node| {
                    !node.is_hidden()
                        && (node.label().as_deref() == Some(expected)
                            || node.value().as_deref() == Some(expected))
                })
                .next()
                .is_some(),
            "missing dimensions panel text: {expected}"
        );
    }
}

#[test]
fn outliner_and_viewport_share_multiselection_without_document_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let revision = app.document_revision();
    let outliner_ids = app
        .outliner_query()
        .into_iter()
        .flat_map(|definition| definition.occurrences)
        .map(|occurrence| occurrence.instance_path)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        outliner_ids,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );

    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.selection.contains(&InstancePath::root(OccurrenceId(1))));
    assert_eq!(app.selection_count(), 1);

    app.select_from_viewport(
        Some(SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        }),
        true,
    );
    assert_eq!(app.selection_count(), 2);
    assert!(app.selection.contains(&InstancePath::root(OccurrenceId(1))));
    assert!(app.selection.contains(&InstancePath::root(OccurrenceId(2))));

    app.select_from_viewport(
        Some(SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Maximum,
            },
        }),
        true,
    );
    assert_eq!(app.selection_count(), 1);

    app.select_from_viewport(None, false);
    assert_eq!(app.selection_count(), 0);
    app.orbit(Vec2::new(18.0, -9.0));
    assert_eq!(app.document_revision(), revision);
}

#[test]
fn shared_definition_push_pull_previews_each_occurrence_and_explains_impact() {
    let mut app = KetchupApp::new();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Box-1 #2".to_owned(),
                transform: Transform::from_translation(250.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.push_pull_distance_input = "60".to_owned();

    assert!(app.start_preview());
    let rendered = app
        .active_boxes()
        .into_iter()
        .map(|item| app.render_box(item))
        .collect::<Vec<_>>();
    assert_eq!(rendered[0].origin_mm, Vec3::ZERO);
    assert_eq!(rendered[1].origin_mm, Vec3::new(250.0, 0.0, 0.0));
    assert_eq!(rendered[0].size_mm.z, 80.0);
    assert_eq!(rendered[1].size_mm.z, 80.0);
    assert!(app.digest.contains("2 occurrence(s) follow"));

    app.cancel_preview();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Minimum,
            },
        },
        false,
    );
    app.push_pull_distance_input = "30".to_owned();
    assert!(app.start_preview());
    let rendered = app
        .active_boxes()
        .into_iter()
        .map(|item| app.render_box(item))
        .collect::<Vec<_>>();
    assert_eq!(rendered[0].origin_mm.x, -30.0);
    assert_eq!(rendered[1].origin_mm.x, 250.0);
    assert_eq!(rendered[0].size_mm.x, 130.0);
    assert_eq!(rendered[1].size_mm.x, 130.0);
}

#[test]
fn exact_rectangle_and_push_pull_are_atomic_undo_steps() {
    let mut app = KetchupApp::new();
    app.dispatch_command(AppCommand::Rectangle);
    app.sketch_start = Some(Vec3::new(40.0, 30.0, 20.0));
    app.sketch_cursor = Some(Vec3::new(20.0, 10.0, 20.0));
    app.value_input = "300,200".to_owned();

    assert!(app.apply_value_input());
    assert_eq!(app.active_box_count(), 2);
    let created = app.active_boxes()[1].clone();
    assert_eq!(created.origin_mm, Vec3::new(-260.0, -170.0, 20.0));
    assert_eq!(created.size_mm, Vec3::new(300.0, 200.0, 0.0));
    assert_eq!(app.document.visible_undo_steps(), 1);

    app.dispatch_command(AppCommand::PushPull);
    app.value_input = "55".to_owned();
    assert!(app.apply_value_input());
    assert_eq!(app.active_boxes()[1].size_mm.z, 55.0);
    assert_eq!(app.document.visible_undo_steps(), 2);

    assert!(app.undo());
    assert_eq!(app.active_boxes()[1].size_mm.z, 0.0);
    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert!(app.redo());
    assert_eq!(app.active_boxes()[1].size_mm.z, 55.0);
}

#[test]
fn move_and_ctrl_copy_commit_occurrence_only_batches_visible_in_outliner() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    let definition_count = app.document.current().definitions().count();

    assert!(app.move_selected(Vec3::new(30.0, 20.0, 0.0)));
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert_eq!(app.outliner_query()[0].occurrences[0].position, "30,20");
    assert_eq!(
        app.document.current().definitions().count(),
        definition_count
    );

    assert!(app.copy_selected(Vec3::new(50.0, 0.0, 0.0)));
    let snapshot = app.document.current();
    assert_eq!(snapshot.definitions().count(), definition_count);
    assert_eq!(snapshot.occurrences().count(), 2);
    assert_eq!(
        snapshot
            .occurrence(OccurrenceId(1))
            .unwrap()
            .definition_id(),
        snapshot
            .occurrence(OccurrenceId(2))
            .unwrap()
            .definition_id()
    );
    assert_eq!(snapshot.scene_query()[0].shared_occurrence_count, 2);
    assert_eq!(
        app.selected_move_reference().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
    assert_eq!(app.outliner_query()[0].occurrences[1].position, "80,20");
    assert_eq!(app.document.visible_undo_steps(), 2);

    assert!(app.undo());
    assert_eq!(app.active_box_count(), 1);
    assert!(app.redo());
    assert_eq!(app.active_box_count(), 2);
    assert_eq!(app.outliner_query()[0].occurrences[1].position, "80,20");
}

#[test]
fn move_vcb_accepts_last_direction_distance_and_exact_vector() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    app.dispatch_command(AppCommand::Move);
    assert!(app.move_selected(Vec3::new(30.0, 40.0, 0.0)));
    app.value_input = "100 mm".to_owned();
    assert!(app.apply_value_input());
    let transform = app
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    assert_eq!(transform.matrix()[3], 60.0);
    assert_eq!(transform.matrix()[7], 80.0);
    assert_eq!(app.document.visible_undo_steps(), 2);

    app.value_input = "10,-20,5".to_owned();
    assert!(app.apply_value_input());
    let transform = app
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    assert_eq!(transform.matrix()[3], 70.0);
    assert_eq!(transform.matrix()[7], 60.0);
    assert_eq!(transform.matrix()[11], 5.0);
}

#[test]
fn adaptive_grid_keeps_metric_lines_readable_across_camera_scales() {
    assert_eq!(adaptive_grid_step(8.0), 10.0);
    assert_eq!(adaptive_grid_step(0.01), 5_000.0);
    assert_eq!(adaptive_grid_step(0.000_01), 5_000_000.0);
    for scale in [8.0, 1.0, 0.01, 0.000_01] {
        let screen_spacing = adaptive_grid_step(scale) * scale;
        assert!(screen_spacing >= 32.0);
        assert!(screen_spacing <= 80.0);
    }
}

#[test]
fn gpu_scene_is_painted_after_the_ground_grid() {
    let mut app = KetchupApp::new();
    let snapshot = app.document.current();
    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact_results,
        &mut app.render_cache,
    ));
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("scene-base-layer-order"),
        ));
        app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
    });

    assert!(output.shapes.len() > 1);
    assert!(matches!(
        output.shapes.last().map(|shape| &shape.shape),
        Some(egui::Shape::Callback(_))
    ));
}

#[test]
fn shadows_are_painted_under_the_gpu_scene_without_grid_dependency() {
    let mut app = KetchupApp::new();
    app.grid_axes_visible = false;
    app.toggle_shadows();
    let boxes = app.active_boxes();
    let snapshot = app.document.current();
    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact_results,
        &mut app.render_cache,
    ));
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("shadow-base-layer-order"),
        ));
        app.paint_projected_shadows(&painter, rect, &boxes);
        app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
    });

    assert_eq!(output.shapes.len(), boxes.len() + 1);
    assert!(matches!(
        output.shapes.last().map(|shape| &shape.shape),
        Some(egui::Shape::Callback(_))
    ));
}

#[test]
fn fog_is_painted_over_the_gpu_scene_as_a_depth_gradient() {
    let mut app = KetchupApp::new();
    app.toggle_fog();
    let snapshot = app.document.current();
    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
        &snapshot,
        &app.exact_results,
        &mut app.render_cache,
    ));
    let context = egui::Context::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("fog-overlay-order"),
        ));
        app.paint_scene_base_layers(&painter, rect, Some(Arc::clone(&plan)));
        app.paint_viewport_fog(&painter, rect);
    });

    let callback_index = output
        .shapes
        .iter()
        .position(|shape| matches!(shape.shape, egui::Shape::Callback(_)))
        .expect("the GPU scene callback must be present");
    assert_eq!(callback_index + 1, output.shapes.len() - 1);
    let Some(egui::Shape::Mesh(haze)) = output.shapes.last().map(|shape| &shape.shape) else {
        panic!("fog must finish with one gradient mesh");
    };
    assert_eq!(haze.vertices.len(), 4);
    assert_eq!(haze.vertices[0].color.a(), 112);
    assert_eq!(haze.vertices[3].color.a(), 8);
}

#[test]
fn xray_projected_faces_use_translucent_fill() {
    let mut app = KetchupApp::new();
    app.toggle_xray();
    let faces = [ProjectedFace {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::GRAY,
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("xray-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });

    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("x-ray projected faces must start with a mesh underlay");
    };
    assert!(
        underlay
            .vertices
            .iter()
            .all(|vertex| vertex.color.a() == 72)
    );
}

#[test]
fn wireframe_projected_faces_emit_no_fill_shapes() {
    let mut app = KetchupApp::new();
    app.toggle_wireframe();
    let faces = [ProjectedFace {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::GRAY,
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("wireframe-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });

    assert!(output.shapes.is_empty());
}

#[test]
fn hidden_edges_emit_no_edge_shapes_but_keep_shaded_face_fills() {
    let mut app = KetchupApp::new();
    app.toggle_edges();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let faces = [ProjectedFace {
        selection: selection.clone(),
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::from_rgb(80, 140, 200),
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let edges = [ProjectedEdge {
        selection,
        points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
        depth: 0.0,
        dominant_axis: None,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("hidden-edge-shaded-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
        app.paint_projected_edges(&painter, &edges);
    });

    assert_eq!(output.shapes.len(), 2);
    assert!(matches!(output.shapes[0].shape, egui::Shape::Mesh(_)));
}

#[test]
fn profiles_use_a_wider_edge_stroke_without_changing_edge_count() {
    let mut app = KetchupApp::new();
    let edges = [ProjectedEdge {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Edge(0),
        },
        points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
        depth: 0.0,
        dominant_axis: None,
    }];
    let context = egui::Context::default();
    let normal = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("normal-profile-stroke"),
            )),
            &edges,
        );
    });
    app.toggle_profiles();
    let emphasized = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("emphasized-profile-stroke"),
            )),
            &edges,
        );
    });

    assert_eq!(normal.shapes.len(), 1);
    assert_eq!(emphasized.shapes.len(), 1);
    let egui::Shape::LineSegment { stroke: normal, .. } = normal.shapes[0].shape else {
        panic!("ordinary profile must be one line segment");
    };
    let egui::Shape::LineSegment {
        stroke: emphasized, ..
    } = emphasized.shapes[0].shape
    else {
        panic!("emphasized profile must be one line segment");
    };
    assert_eq!(normal.width, 1.25);
    assert_eq!(emphasized.width, 2.75);
}

#[test]
fn halos_underpaint_each_edge_with_fixed_screen_space_width_and_compose_with_dashes() {
    let mut app = KetchupApp::new();
    app.toggle_halos();
    let edge = ProjectedEdge {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Edge(0),
        },
        points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
        depth: 0.0,
        dominant_axis: Some(Axis::X),
    };
    let context = egui::Context::default();
    let solid = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("solid-edge-halo"),
            )),
            std::slice::from_ref(&edge),
        );
    });

    assert_eq!(solid.shapes.len(), 2);
    assert!(matches!(
        solid.shapes[0].shape,
        egui::Shape::LineSegment { points, stroke }
            if points == edge.points
                && stroke.width == 4.25
                && stroke.color == Color32::from_rgb(24, 28, 35)
    ));
    assert!(matches!(
        solid.shapes[1].shape,
        egui::Shape::LineSegment { points, stroke }
            if points == edge.points
                && stroke.width == 1.25
                && stroke.color == Color32::from_rgb(182, 192, 207)
    ));

    app.toggle_dashes();
    app.toggle_color_by_axis();
    let dashed = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("dashed-edge-halo"),
            )),
            std::slice::from_ref(&edge),
        );
    });
    assert_eq!(dashed.shapes.len(), 6);
    assert!(dashed.shapes[..3].iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 4.25
                && stroke.color == Color32::from_rgb(24, 28, 35)
    )));
    assert!(dashed.shapes[3..].iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 1.25 && stroke.color == axis_color(Axis::X)
    )));
}

#[test]
fn depth_cue_weights_near_edges_more_than_far_edges_without_reordering() {
    let mut app = KetchupApp::new();
    app.toggle_depth_cue();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let near_points = [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)];
    let far_points = [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)];
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: near_points,
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: far_points,
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("depth-cue-strokes"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 2);
    let egui::Shape::LineSegment {
        points: painted_near,
        stroke: near_stroke,
    } = output.shapes[0].shape
    else {
        panic!("near edge must remain the first line segment");
    };
    let egui::Shape::LineSegment {
        points: painted_far,
        stroke: far_stroke,
    } = output.shapes[1].shape
    else {
        panic!("far edge must remain the second line segment");
    };
    assert_eq!(painted_near, near_points);
    assert_eq!(painted_far, far_points);
    assert!(near_stroke.width > far_stroke.width);
    assert!((near_stroke.width - 1.6).abs() < f32::EPSILON * 2.0);
    assert!((far_stroke.width - 0.9).abs() < f32::EPSILON * 2.0);
}

#[test]
fn distant_edge_fade_blends_far_axis_color_into_current_background() {
    let mut app = KetchupApp::new();
    app.toggle_fade_distant_edges();
    app.toggle_color_by_axis();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: Some(Axis::X),
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)],
            depth: 10.0,
            dominant_axis: Some(Axis::X),
        },
    ];
    let context = egui::Context::default();
    let paint = |app: &KetchupApp, id: &'static str| {
        context.run(egui::RawInput::default(), |context| {
            app.paint_projected_edges(
                &context.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id))),
                &edges,
            );
        })
    };

    let dark = paint(&app, "dark-distant-edge-fade");
    assert_eq!(dark.shapes.len(), 2);
    assert!(matches!(
        dark.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == axis_color(Axis::X)
    ));
    assert!(matches!(
        dark.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(94, 48, 44)
    ));

    app.toggle_white_background();
    let white = paint(&app, "white-distant-edge-fade");
    assert_eq!(white.shapes.len(), 2);
    assert!(matches!(
        white.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(237, 190, 184)
    ));
}

#[test]
fn high_contrast_edges_override_axis_color_and_fade_into_each_background() {
    let mut app = KetchupApp::new();
    app.toggle_high_contrast_edges();
    app.toggle_color_by_axis();
    app.toggle_fade_distant_edges();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: Some(Axis::X),
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)],
            depth: 10.0,
            dominant_axis: Some(Axis::X),
        },
    ];
    let context = egui::Context::default();
    let paint = |app: &KetchupApp, id: &'static str| {
        context.run(egui::RawInput::default(), |context| {
            app.paint_projected_edges(
                &context.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id))),
                &edges,
            );
        })
    };

    let dark = paint(&app, "dark-high-contrast-edges");
    assert_eq!(dark.shapes.len(), 2);
    assert!(matches!(
        dark.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == Color32::WHITE
    ));
    assert!(matches!(
        dark.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(104, 107, 112)
    ));

    app.toggle_white_background();
    let white = paint(&app, "white-high-contrast-edges");
    assert_eq!(white.shapes.len(), 2);
    assert!(matches!(
        white.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == Color32::BLACK
    ));
    assert!(matches!(
        white.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == Color32::from_rgb(158, 160, 162)
    ));
}

#[test]
fn selection_halo_underpaints_selected_edges_with_background_contrast() {
    let mut app = KetchupApp::new();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    app.selection.select_exact(selection.clone(), false);
    let edge = ProjectedEdge {
        selection,
        points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
        depth: 0.0,
        dominant_axis: None,
    };
    let context = egui::Context::default();
    let paint = |app: &KetchupApp, id: &'static str| {
        context.run(egui::RawInput::default(), |context| {
            app.paint_projected_selection(
                &context.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id))),
                std::slice::from_ref(&edge),
            );
        })
    };

    let ordinary = paint(&app, "ordinary-selection");
    assert_eq!(ordinary.shapes.len(), 1);
    assert!(matches!(
        ordinary.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 1.8 && stroke.color == Color32::from_rgb(240, 78, 35)
    ));

    app.toggle_selection_halo();
    let dark = paint(&app, "dark-selection-halo");
    assert_eq!(dark.shapes.len(), 2);
    assert!(matches!(
        dark.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 4.8 && stroke.color == Color32::WHITE
    ));
    assert!(matches!(
        dark.shapes[1].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 1.8 && stroke.color == Color32::from_rgb(240, 78, 35)
    ));

    app.toggle_white_background();
    let white = paint(&app, "white-selection-halo");
    assert_eq!(white.shapes.len(), 2);
    assert!(matches!(
        white.shapes[0].shape,
        egui::Shape::LineSegment { stroke, .. }
            if stroke.width == 4.8 && stroke.color == Color32::BLACK
    ));
}

#[test]
fn endpoints_follow_all_edge_strokes_with_exactly_two_markers_per_edge() {
    let mut app = KetchupApp::new();
    app.toggle_endpoints();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let first_points = [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)];
    let second_points = [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)];
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: first_points,
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: second_points,
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("endpoint-markers"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 6);
    assert!(matches!(
        output.shapes[0].shape,
        egui::Shape::LineSegment { points, .. } if points == first_points
    ));
    assert!(matches!(
        output.shapes[1].shape,
        egui::Shape::LineSegment { points, .. } if points == second_points
    ));
    for (shape, expected) in output.shapes[2..].iter().zip([
        first_points[0],
        first_points[1],
        second_points[0],
        second_points[1],
    ]) {
        let egui::Shape::Circle(marker) = &shape.shape else {
            panic!("endpoint marker must follow every edge stroke");
        };
        assert_eq!(marker.center, expected);
        assert_eq!(marker.radius, 3.25);
        assert_eq!(marker.fill, Color32::from_rgb(232, 158, 72));
    }
}

#[test]
fn midpoints_follow_jittered_edges_once_and_compose_with_endpoints_and_dashes() {
    let mut app = KetchupApp::new();
    app.toggle_midpoints();
    app.toggle_jitter();
    app.toggle_endpoints();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: Some(Axis::X),
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(10.0, 30.0), Pos2::new(110.0, 30.0)],
            depth: 10.0,
            dominant_axis: Some(Axis::X),
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-midpoints"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 8);
    for (shape, expected) in output.shapes[2..4]
        .iter()
        .zip([Pos2::new(58.5, 9.25), Pos2::new(61.5, 29.25)])
    {
        let egui::Shape::Circle(marker) = &shape.shape else {
            panic!("each edge must receive exactly one midpoint marker");
        };
        assert_eq!(marker.center, expected);
        assert_eq!(marker.radius, 3.25);
        assert_eq!(marker.fill, Color32::from_rgb(96, 201, 138));
    }
    assert!(
        output.shapes[4..]
            .iter()
            .all(|shape| matches!(shape.shape, egui::Shape::Circle(_)))
    );

    app.toggle_jitter();
    app.toggle_endpoints();
    app.toggle_dashes();
    app.toggle_color_by_axis();
    let dashed = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("dashed-edge-midpoint"),
            )),
            &[ProjectedEdge {
                selection: edges[0].selection.clone(),
                points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
                depth: 0.0,
                dominant_axis: Some(Axis::X),
            }],
        );
    });
    assert_eq!(dashed.shapes.len(), 4);
    assert!(dashed.shapes[..3].iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == axis_color(Axis::X)
    )));
    assert!(matches!(
        dashed.shapes[3].shape,
        egui::Shape::Circle(ref marker)
            if marker.center == Pos2::new(20.0, 10.0)
                && marker.fill == Color32::from_rgb(96, 201, 138)
    ));
}

#[test]
fn extensions_follow_all_edge_strokes_with_fixed_screen_space_overhangs() {
    let mut app = KetchupApp::new();
    app.toggle_extensions();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let horizontal = [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)];
    let vertical = [Pos2::new(30.0, 20.0), Pos2::new(30.0, 120.0)];
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: horizontal,
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: vertical,
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-extensions"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 6);
    assert!(matches!(
        output.shapes[0].shape,
        egui::Shape::LineSegment { points, .. } if points == horizontal
    ));
    assert!(matches!(
        output.shapes[1].shape,
        egui::Shape::LineSegment { points, .. } if points == vertical
    ));
    for (shape, expected) in output.shapes[2..].iter().zip([
        [Pos2::new(3.0, 10.0), Pos2::new(10.0, 10.0)],
        [Pos2::new(110.0, 10.0), Pos2::new(117.0, 10.0)],
        [Pos2::new(30.0, 13.0), Pos2::new(30.0, 20.0)],
        [Pos2::new(30.0, 120.0), Pos2::new(30.0, 127.0)],
    ]) {
        assert!(matches!(
            shape.shape,
            egui::Shape::LineSegment { points, .. } if points == expected
        ));
    }
}

#[test]
fn jitter_offsets_edge_strokes_deterministically_without_reordering() {
    let mut app = KetchupApp::new();
    app.toggle_jitter();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(110.0, 10.0)],
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(30.0, 20.0), Pos2::new(30.0, 120.0)],
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-jitter"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 2);
    assert!(matches!(
        output.shapes[0].shape,
        egui::Shape::LineSegment { points, .. }
            if points == [Pos2::new(8.5, 9.25), Pos2::new(108.5, 9.25)]
    ));
    assert!(matches!(
        output.shapes[1].shape,
        egui::Shape::LineSegment { points, .. }
            if points == [Pos2::new(31.5, 19.25), Pos2::new(31.5, 119.25)]
    ));
}

#[test]
fn dashes_split_each_edge_with_fixed_screen_space_rhythm_without_reordering() {
    let mut app = KetchupApp::new();
    app.toggle_dashes();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [
        ProjectedEdge {
            selection: selection.clone(),
            points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
            depth: 0.0,
            dominant_axis: None,
        },
        ProjectedEdge {
            selection,
            points: [Pos2::new(40.0, 20.0), Pos2::new(40.0, 40.0)],
            depth: 10.0,
            dominant_axis: None,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("edge-dashes"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 6);
    for (shape, expected) in output.shapes.iter().zip([
        [Pos2::new(10.0, 10.0), Pos2::new(15.0, 10.0)],
        [Pos2::new(19.0, 10.0), Pos2::new(24.0, 10.0)],
        [Pos2::new(28.0, 10.0), Pos2::new(30.0, 10.0)],
        [Pos2::new(40.0, 20.0), Pos2::new(40.0, 25.0)],
        [Pos2::new(40.0, 29.0), Pos2::new(40.0, 34.0)],
        [Pos2::new(40.0, 38.0), Pos2::new(40.0, 40.0)],
    ]) {
        assert!(matches!(
            shape.shape,
            egui::Shape::LineSegment { points, .. } if points == expected
        ));
    }
}

#[test]
fn color_by_axis_classifies_world_edges_and_colors_strokes_and_extensions() {
    assert_eq!(
        dominant_edge_axis([Vec3::ZERO, Vec3::new(12.0, 3.0, 1.0)]),
        Some(Axis::X)
    );
    assert_eq!(
        dominant_edge_axis([Vec3::ZERO, Vec3::new(2.0, -8.0, 4.0)]),
        Some(Axis::Y)
    );
    assert_eq!(
        dominant_edge_axis([Vec3::ZERO, Vec3::new(1.0, 2.0, -9.0)]),
        Some(Axis::Z)
    );
    assert_eq!(dominant_edge_axis([Vec3::ZERO, Vec3::ZERO]), None);

    let mut app = KetchupApp::new();
    app.toggle_color_by_axis();
    app.toggle_extensions();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Edge(0),
    };
    let edges = [Axis::X, Axis::Y, Axis::Z].map(|axis| ProjectedEdge {
        selection: selection.clone(),
        points: [Pos2::new(10.0, 10.0), Pos2::new(30.0, 10.0)],
        depth: 0.0,
        dominant_axis: Some(axis),
    });
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("color-by-axis"),
            )),
            &edges,
        );
    });

    assert_eq!(output.shapes.len(), 9);
    let colors = output
        .shapes
        .iter()
        .map(|shape| match shape.shape {
            egui::Shape::LineSegment { stroke, .. } => stroke.color,
            _ => panic!("axis-colored edge must remain a line segment"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        colors[..3],
        [
            axis_color(Axis::X),
            axis_color(Axis::Y),
            axis_color(Axis::Z)
        ]
    );
    assert_eq!(
        colors[3..],
        [
            axis_color(Axis::X),
            axis_color(Axis::X),
            axis_color(Axis::Y),
            axis_color(Axis::Y),
            axis_color(Axis::Z),
            axis_color(Axis::Z),
        ]
    );

    app.toggle_extensions();
    app.toggle_dashes();
    let dashed = context.run(egui::RawInput::default(), |context| {
        app.paint_projected_edges(
            &context.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("color-by-axis-dashes"),
            )),
            &edges[..1],
        );
    });
    assert_eq!(dashed.shapes.len(), 3);
    assert!(dashed.shapes.iter().all(|shape| matches!(
        shape.shape,
        egui::Shape::LineSegment { stroke, .. } if stroke.color == axis_color(Axis::X)
    )));
}

#[test]
fn monochrome_projected_faces_are_grayscale_but_selection_feedback_stays_colored() {
    let mut app = KetchupApp::new();
    app.toggle_monochrome();
    let faces = [ProjectedFace {
        selection: SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        polygon: ProjectedPolygon::Triangle([
            Pos2::new(10.0, 10.0),
            Pos2::new(110.0, 10.0),
            Pos2::new(10.0, 110.0),
        ]),
        color: Color32::from_rgb(80, 140, 200),
        depth: 0.0,
        previewed: false,
        out_of_context: false,
    }];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("monochrome-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("monochrome projected faces must start with a mesh underlay");
    };
    assert!(underlay.vertices.iter().all(|vertex| {
        vertex.color.r() == vertex.color.g() && vertex.color.g() == vertex.color.b()
    }));

    app.selection.select_occurrence(OccurrenceId(1), false);
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("monochrome-selected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("selected monochrome faces must start with a mesh underlay");
    };
    assert!(
        underlay
            .vertices
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(154, 91, 67))
    );
}

#[test]
fn hidden_line_projected_faces_are_flat_neutral_but_selection_feedback_stays_colored() {
    let mut app = KetchupApp::new();
    app.toggle_hidden_line();
    let faces = [
        ProjectedFace {
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(10.0, 10.0),
                Pos2::new(110.0, 10.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::from_rgb(80, 140, 200),
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
        ProjectedFace {
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(2)),
                element: ElementId::Face {
                    axis: Axis::X,
                    side: Side::Maximum,
                },
            },
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(120.0, 10.0),
                Pos2::new(220.0, 10.0),
                Pos2::new(120.0, 110.0),
            ]),
            color: Color32::from_rgb(200, 90, 40),
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("hidden-line-projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("hidden-line projected faces must start with a mesh underlay");
    };
    assert!(
        underlay
            .vertices
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(214, 218, 224))
    );

    app.selection.select_occurrence(OccurrenceId(1), false);
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("hidden-line-selected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("selected hidden-line faces must start with a mesh underlay");
    };
    assert!(
        underlay.vertices[..3]
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(154, 91, 67))
    );
    assert!(
        underlay.vertices[3..]
            .iter()
            .all(|vertex| vertex.color == Color32::from_rgb(214, 218, 224))
    );
}

#[test]
fn adjacent_projected_triangles_share_a_fill_underlay_and_keep_antialiased_outlines() {
    let app = KetchupApp::new();
    let selection = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    let faces = vec![
        ProjectedFace {
            selection: selection.clone(),
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(10.0, 10.0),
                Pos2::new(110.0, 10.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::GRAY,
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
        ProjectedFace {
            selection,
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(110.0, 10.0),
                Pos2::new(110.0, 110.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::GRAY,
            depth: 0.0,
            previewed: false,
            out_of_context: false,
        },
    ];
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("projected-face-fill"),
        ));
        app.paint_projected_faces(&painter, &faces);
    });

    assert_eq!(output.shapes.len(), 3);
    let egui::Shape::Mesh(underlay) = &output.shapes[0].shape else {
        panic!("projected faces must start with one shared mesh underlay");
    };
    assert_eq!(underlay.vertices.len(), 6);
    assert_eq!(underlay.indices.len(), 6);
    assert!(
        output.shapes[1..]
            .iter()
            .all(|shape| matches!(shape.shape, egui::Shape::Path(_)))
    );
}

#[test]
fn move_drag_snaps_to_grid_and_shift_constrains_dominant_axis() {
    let start = Vec3::new(3.0, 4.0, 20.0);
    assert_eq!(
        snapped_move_delta(start, Vec3::new(31.0, 28.0, 20.0), false),
        Vec3::new(30.0, 20.0, 0.0)
    );
    assert_eq!(
        snapped_move_delta(start, Vec3::new(31.0, 28.0, 20.0), true),
        Vec3::new(30.0, 0.0, 0.0)
    );
}

#[test]
fn group_and_ungroup_preserve_world_placement_as_atomic_batches() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    let before = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .map(|item| (item.occurrence_id, item.transform))
        .collect::<BTreeMap<_, _>>();
    let undo_steps = app.document.visible_undo_steps();

    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let grouped = app.document.current();
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(grouped.groups().count(), 1);
    assert_eq!(
        grouped.occurrence(OccurrenceId(1)).unwrap().parent(),
        Some(group_id)
    );
    assert_eq!(
        grouped.occurrence(OccurrenceId(2)).unwrap().parent(),
        Some(group_id)
    );
    assert_eq!(
        grouped
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>(),
        before
    );

    assert!(app.undo());
    assert_eq!(app.document.current().groups().count(), 0);
    assert!(app.redo());
    assert!(app.select_group(group_id));
    assert!(app.ungroup_selected());
    assert_eq!(app.document.current().groups().count(), 0);
    assert_eq!(
        app.document
            .current()
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>(),
        before
    );
    assert!(app.undo());
    assert_eq!(app.document.current().groups().count(), 1);
    assert!(app.redo());
    assert_eq!(app.document.current().groups().count(), 0);
}

#[test]
fn ungroup_composes_parent_transform_into_occurrences_and_child_groups() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let parent_group = app.selection.selected_group.unwrap();
    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(4)), true);
    assert!(app.group_selected());
    let child_group = app.selection.selected_group.unwrap();
    let parent_transform = Transform::from_translation(40.0, -20.0, 15.0).unwrap();
    let child_transform = Transform::from_translation(-5.0, 12.0, 3.0).unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: parent_group,
                transform: parent_transform,
            },
            CanonicalCommand::SetGroupTransform {
                id: child_group,
                transform: child_transform,
            },
            CanonicalCommand::SetGroupParent {
                id: child_group,
                parent: Some(parent_group),
            },
        ]))
        .unwrap();
    let before = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .map(|item| (item.occurrence_id, item.transform))
        .collect::<BTreeMap<_, _>>();
    assert!(app.select_group(parent_group));
    let undo_steps = app.document.visible_undo_steps();

    assert!(app.ungroup_selected());

    let snapshot = app.document.current();
    assert!(snapshot.group(parent_group).is_none());
    let child = snapshot.group(child_group).unwrap();
    assert_eq!(child.parent(), None);
    assert_eq!(child.transform(), parent_transform.compose(child_transform));
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(
        snapshot
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>(),
        before
    );
    assert!(app.undo());
    assert!(app.document.current().group(parent_group).is_some());
    assert!(app.redo());
    assert!(app.document.current().group(parent_group).is_none());
}

#[test]
fn make_component_converts_a_nested_group_subtree_in_one_undo_step() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let parent_group = app.selection.selected_group.unwrap();
    let child_group = GroupId(10);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: child_group,
                name: "Child".to_owned(),
                transform: Transform::identity(),
                parent: Some(parent_group),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(3),
                parent: Some(child_group),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(4),
                parent: Some(child_group),
            },
        ]))
        .unwrap();
    assert!(app.select_group(parent_group));
    let before = app.canonical_digest();
    let revision = app.document_revision();
    let undo_steps = app.document.visible_undo_steps();
    assert!(app.make_component_source_plan().is_some());

    assert!(app.make_component());

    assert_eq!(app.group_count(), 0);
    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-made-component",
            &BTreeMap::from([
                (
                    "name",
                    app.catalog.format(
                        "model-component-name",
                        &BTreeMap::from([("number", parent_group.0.to_string())]),
                    )
                ),
                ("count", "4".to_owned()),
            ]),
        )
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before);
    assert_eq!(app.group_count(), 2);
    assert_eq!(app.occurrence_count(), 4);
    assert!(app.redo());
    assert_eq!(app.group_count(), 0);
    assert_eq!(app.occurrence_count(), 1);
}

#[test]
fn make_component_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let plan = app.make_component_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(plan.group_id, group_id);
    assert_eq!(plan.occurrence_count, 2);
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(plan.primary, None);
    assert_eq!(plan.selected_group, Some(group_id));
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.subtree_occurrence_count, 2);
    assert_eq!(plan.new_definition_id, DefinitionId(3));
    assert_eq!(plan.new_occurrence_id, OccurrenceId(3));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    let mut tampered = plan.clone();
    tampered.group_name.push('!');
    assert!(!app.apply_make_component_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(group_id));
    assert!(!app.apply_make_component_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.create_box());
    assert!(app.select_group(group_id));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_make_component_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn make_component_rejects_a_nested_local_id_collision_without_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let parent_group = app.selection.selected_group.unwrap();
    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(4)), true);
    assert!(app.group_selected());
    let child_group = app.selection.selected_group.unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetGroupParent {
            id: child_group,
            parent: Some(parent_group),
        }]))
        .unwrap();
    assert!(app.select_group(parent_group));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.document.visible_undo_steps();

    assert!(app.make_component_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);
}

#[test]
fn component_copies_keep_distinct_nested_paths_and_composed_world_positions() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let before = app
        .active_boxes()
        .into_iter()
        .map(|item| item.origin_mm)
        .collect::<Vec<_>>();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());

    let converted = app.active_boxes();
    assert_eq!(
        converted
            .iter()
            .map(|item| item.origin_mm)
            .collect::<Vec<_>>(),
        before
    );
    assert!(converted.iter().all(|item| !item.instance_path.is_root()));
    let component_path = app.selected_move_reference().unwrap().instance_path;
    assert!(component_path.is_root());
    assert!(app.copy_selected(Vec3::new(200.0, 0.0, 0.0)));

    let boxes = app.active_boxes();
    let paths = boxes
        .iter()
        .map(|item| item.instance_path.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(paths.len(), 4);
    assert_eq!(
        paths
            .iter()
            .map(InstancePath::root_occurrence)
            .collect::<BTreeSet<_>>()
            .len(),
        2
    );
    let mut expected = before.clone();
    expected.extend(
        before
            .iter()
            .map(|origin| *origin + Vec3::new(200.0, 0.0, 0.0)),
    );
    let actual = boxes.iter().map(|item| item.origin_mm).collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn purge_preserves_definitions_referenced_by_nested_component_occurrences() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert_eq!(app.definition_count(), 3);
    assert_eq!(app.purgeable_definition_count(), 0);

    assert!(app.delete_selected());
    assert_eq!(app.occurrence_count(), 0);
    assert_eq!(app.purgeable_definition_count(), 0);
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    assert!(!app.purge_unused_definitions());
    assert_eq!(app.definition_count(), 3);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
}

#[test]
fn purge_source_plan_is_exact_stale_safe_and_one_undo_step() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let expected_definition_ids = app
        .document
        .current()
        .definitions()
        .map(|definition| definition.id())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_definition_ids.len(), 2);
    assert!(app.select_all());
    assert!(app.delete_selected());

    let plan = app.purge_unused_source_plan().unwrap();
    assert_eq!(plan.definition_ids, expected_definition_ids);
    assert_eq!(plan.definition_count, 2);
    let stale_plan = plan.clone();
    assert!(app.undo());
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    assert!(!app.apply_purge_unused_source_plan(stale_plan));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.redo());
    let before_purge_steps = app.undo_step_count();
    assert!(app.apply_purge_unused_source_plan(plan));
    assert_eq!(app.definition_count(), 0);
    assert_eq!(app.undo_step_count(), before_purge_steps + 1);
    assert!(app.undo());
    assert_eq!(app.definition_count(), 2);
    assert!(app.redo());
    assert_eq!(app.definition_count(), 0);
}

#[test]
fn nested_edits_require_a_matching_snapshot_bound_context() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    let nested = app.active_boxes()[0].clone();
    let selection = SelectionId {
        definition_id: nested.definition_id,
        instance_path: nested.instance_path.clone(),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.select_exact(selection.clone(), false);
    let before_revision = app.document_revision();
    let before_digest = app.document.current().canonical_digest();
    app.set_push_pull_distance_input("5");
    assert!(!app.start_preview());
    assert!(!app.move_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.document.current().canonical_digest(), before_digest);

    let component_path = InstancePath::root(nested.instance_path.root_occurrence());
    assert!(app.enter_occurrence_context(component_path));
    app.selection.select_exact(selection.clone(), false);
    assert!(app.start_preview());
    assert!(app.preview_action_digest().is_some());
    assert!(app.confirm_preview());
    let committed_digest = app.document.current().canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(app.document_revision(), before_revision + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.document.current().canonical_digest(), committed_digest);

    app.selection.select_exact(selection, false);
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    let stale_revision = app.document_revision();
    let stale_digest = app.document.current().canonical_digest();
    assert!(app.exit_edit_context());
    assert!(!app.confirm_preview());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.document.current().canonical_digest(), stale_digest);
}

#[test]
fn edit_context_blocks_selection_leakage_and_exits_one_level_at_a_time() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();

    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert_eq!(app.selection.selected_group, Some(group_id));
    assert!(app.enter_occurrence_context(InstancePath::root(OccurrenceId(1))));
    assert_eq!(
        app.selection.edit_context,
        vec![EditContext::Group(group_id)]
    );

    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    assert_eq!(app.selection_count(), 0);
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert_eq!(app.selection_count(), 1);
    assert!(app.enter_occurrence_context(InstancePath::root(OccurrenceId(1))));
    assert!(matches!(
        app.selection.edit_context.last(),
        Some(EditContext::Definition {
            definition_id: DefinitionId(1),
            instance_path,
        }) if *instance_path == InstancePath::root(OccurrenceId(1))
    ));

    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    assert_eq!(app.selection_count(), 0);
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert_eq!(app.selection_count(), 1);
    let tag = TagId(91_001);
    assert!(
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: tag,
                    name: "Local guard".to_owned(),
                    visible: true,
                },
                CanonicalCommand::SetOccurrenceTag {
                    id: OccurrenceId(1),
                    tag: Some(tag),
                },
            ]))
            .is_ok()
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let local_selection = app.selected_occurrence_ids();
    app.begin_tag_creation(Some(local_selection));
    assert!(!app.tag_creation_visible());
    assert!(!app.assign_selection_to_tag(tag));
    assert!(!app.remove_selection_from_tag(tag));
    assert!(!app.isolate_selected_tags());
    assert!(!app.hide_selected_tags());
    assert!(!app.show_selected_tags());
    assert!(!app.invert_selected_tags());
    assert!(!app.select_matching_tags());
    assert!(!app.select_tag_occurrences(tag));
    assert!(!app.select_all_tagged_occurrences());
    assert!(!app.select_untagged_occurrences());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    app.clear_selection();
    assert!(app.exit_edit_context());
    assert_eq!(
        app.selection.edit_context,
        vec![EditContext::Group(group_id)]
    );
    assert!(app.exit_edit_context());
    assert!(app.selection.edit_context.is_empty());
}

#[test]
fn used_local_tag_deletion_fails_closed_without_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let tag = TagId(91_002);
    assert!(
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: tag,
                    name: "Local delete guard".to_owned(),
                    visible: true,
                },
                CanonicalCommand::SetOccurrenceTag {
                    id: OccurrenceId(1),
                    tag: Some(tag),
                },
            ]))
            .is_ok()
    );
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert!(!app.can_delete_tag(tag));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    app.begin_tag_deletion(tag);
    app.begin_tag_clear(tag);

    assert!(!app.tag_deletion_visible());
    assert!(!app.confirm_tag_deletion());
    assert!(!app.tag_clear_visible());
    assert!(!app.confirm_tag_clear());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert!(app.document.current().tag(tag).is_some());
    assert!(
        app.document
            .current()
            .local_occurrences()
            .any(|occurrence| occurrence.tag() == Some(tag))
    );
}

#[test]
fn copy_plan_is_exact_document_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    let plan = app.copy_source_plan().unwrap();
    assert_eq!(
        plan.occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.occurrence_count, 2);
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_copy_source_plan(plan.clone()));

    assert_eq!(
        app.occurrence_clipboard,
        vec![OccurrenceId(1), OccurrenceId(2)]
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    let action_digest = app.action_digest().to_owned();

    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(app.copy_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Copy));
    assert!(!app.apply_copy_source_plan(plan));
    assert_eq!(
        app.occurrence_clipboard,
        vec![OccurrenceId(1), OccurrenceId(2)]
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);

    app.selection.selected_group = Some(GroupId(999));
    assert!(app.copy_source_plan().is_none());
}

#[test]
fn cut_plan_is_exact_atomic_pasteable_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    let plan = app.cut_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.occurrence_count, 2);
    assert_eq!(plan.clipboard.len(), 2);
    assert_eq!(plan.commands.len(), 2);
    let before_cut = app.canonical_digest();
    let revision = app.document_revision();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_cut_source_plan(plan));

    assert_eq!(app.occurrence_count(), 0);
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(
        app.occurrence_clipboard,
        vec![OccurrenceId(1), OccurrenceId(2)]
    );
    assert_eq!(app.cut_occurrence_clipboard.len(), 2);
    assert!(app.command_enabled(AppCommand::Paste));
    assert!(app.paste_clipboard());
    assert_eq!(app.occurrence_count(), 2);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert!(app.undo());
    assert_eq!(app.occurrence_count(), 0);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_cut);

    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(1)),
        InstancePath::root(OccurrenceId(2)),
    ]);
    let stale_plan = app.cut_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_clipboard = app.occurrence_clipboard.clone();
    assert!(!app.apply_cut_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.occurrence_clipboard, stale_clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.cut_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Cut));
}

#[test]
fn paste_plan_is_exact_atomic_context_bound_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    assert!(app.copy_selection_to_clipboard());
    let plan = app.paste_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.source_occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.source_occurrence_count, 2);
    assert_eq!(plan.commands.len(), 2);
    assert_eq!(
        plan.pasted
            .iter()
            .map(|(occurrence_id, _)| *occurrence_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([OccurrenceId(3), OccurrenceId(4)])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_paste_source_plan(plan));

    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.occurrence_count(), 4);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.occurrence_count(), 4);

    let stale_plan = app.paste_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_paste_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.paste_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Paste));
    assert!(!app.paste_clipboard());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    app.selection.edit_context.clear();

    app.occurrence_clipboard = vec![OccurrenceId(2), OccurrenceId(1)];
    assert!(app.paste_source_plan().is_none());
    app.occurrence_clipboard = vec![OccurrenceId(1), OccurrenceId(1)];
    assert!(app.paste_source_plan().is_none());
}

#[test]
fn duplicate_plan_is_exact_atomic_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    let plan = app.duplicate_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.source_occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.source_occurrence_count, 2);
    assert_eq!(plan.commands.len(), 2);
    assert_eq!(
        plan.duplicated
            .iter()
            .map(|(occurrence_id, _)| *occurrence_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([OccurrenceId(3), OccurrenceId(4)])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_duplicate_source_plan(plan));

    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.occurrence_count(), 4);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.occurrence_clipboard, clipboard);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.occurrence_count(), 4);
    assert_eq!(app.occurrence_clipboard, clipboard);
    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(4)), true);

    let stale_plan = app.duplicate_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_duplicate_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection.selected_group = Some(GroupId(999));
    assert!(app.duplicate_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Duplicate));
    app.selection.selected_group = None;
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.duplicate_source_plan().is_none());
    assert!(!app.duplicate_selection());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn delete_plan_is_exact_atomic_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(3)));
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    let plan = app.delete_selection_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.occurrence_count, 2);
    assert!(plan.group_ids.is_empty());
    assert_eq!(plan.group_count, 0);
    assert_eq!(plan.commands.len(), 2);
    let before_delete = app.canonical_digest();
    let revision = app.document_revision();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_delete_selection_source_plan(plan));

    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.occurrence_clipboard, clipboard);
    assert!(app.command_enabled(AppCommand::Paste));
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-deleted",
            &BTreeMap::from([("count", "2".to_owned())]),
        )
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_delete);
    assert_eq!(app.occurrence_clipboard, clipboard);
    assert!(app.redo());
    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.undo());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(1)),
        InstancePath::root(OccurrenceId(2)),
    ]);
    let stale_plan = app.delete_selection_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_delete_selection_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(app.delete_selection_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Delete));
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.delete_selection_source_plan().is_none());
    assert!(!app.delete_selected());

    let mut group_app = KetchupApp::new();
    assert!(group_app.create_box());
    group_app.selection.clear();
    group_app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    assert!(group_app.group_selected());
    let group_plan = group_app.delete_selection_source_plan().unwrap();
    assert_eq!(group_plan.source_revision, group_app.document_revision());
    assert_eq!(group_plan.occurrence_count, 2);
    assert_eq!(group_plan.group_count, 1);
    assert_eq!(group_plan.commands.len(), 3);
    assert!(group_app.apply_delete_selection_source_plan(group_plan));
    assert_eq!(group_app.occurrence_count(), 0);
    assert_eq!(group_app.group_count(), 0);
    assert!(group_app.undo());
    assert_eq!(group_app.occurrence_count(), 2);
    assert_eq!(group_app.group_count(), 1);
}

#[test]
fn deselect_plan_is_exact_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection.selected_group = Some(GroupId(999));
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(777)));
    let edit_context = app.selection.edit_context.clone();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    app.occurrence_clipboard = vec![OccurrenceId(1)];
    let clipboard = app.occurrence_clipboard.clone();
    let plan = app.deselect_source_plan().unwrap();
    assert_eq!(plan.source_revision, revision);
    assert!(plan.occurrence_paths.is_empty());
    assert_eq!(plan.occurrence_count, 0);
    assert!(plan.primary.is_none());
    assert_eq!(plan.selected_group, Some(GroupId(999)));
    assert_eq!(plan.edit_context, edit_context);
    assert!(app.command_enabled(AppCommand::Deselect));

    assert!(app.clear_selection());

    assert!(app.selection.occurrences.is_empty());
    assert!(app.selection.primary.is_none());
    assert!(app.selection.selected_group.is_none());
    assert_eq!(app.selection.edit_context, edit_context);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.occurrence_clipboard, clipboard);
    assert!(!app.command_enabled(AppCommand::Deselect));
    let action_digest = app.action_digest().to_owned();
    assert!(!app.clear_selection());
    assert_eq!(app.action_digest(), action_digest);

    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(app.deselect_source_plan().is_some());
    assert!(app.clear_selection());
    assert!(app.selection.occurrences.is_empty());
    assert_eq!(app.selection.edit_context, edit_context);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut stale_app = KetchupApp::new();
    stale_app.selection.clear();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let stale_revision_plan = stale_app.deselect_source_plan().unwrap();
    assert!(stale_app.create_box());
    let stale_revision = stale_app.document_revision();
    let stale_digest = stale_app.canonical_digest();
    let stale_undo_steps = stale_app.undo_step_count();
    let stale_action_digest = stale_app.action_digest().to_owned();
    let stale_occurrences = stale_app.selection.occurrences.clone();
    let stale_primary = stale_app.selection.primary.clone();
    let stale_selected_group = stale_app.selection.selected_group;
    let stale_edit_context = stale_app.selection.edit_context.clone();
    assert!(!stale_app.apply_deselect_source_plan(stale_revision_plan));
    assert_eq!(stale_app.document_revision(), stale_revision);
    assert_eq!(stale_app.canonical_digest(), stale_digest);
    assert_eq!(stale_app.undo_step_count(), stale_undo_steps);
    assert_eq!(stale_app.action_digest(), stale_action_digest);
    assert_eq!(stale_app.selection.occurrences, stale_occurrences);
    assert_eq!(stale_app.selection.primary, stale_primary);
    assert_eq!(stale_app.selection.selected_group, stale_selected_group);
    assert_eq!(stale_app.selection.edit_context, stale_edit_context);

    stale_app.selection.clear();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let stale_selection_plan = stale_app.deselect_source_plan().unwrap();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(!stale_app.apply_deselect_source_plan(stale_selection_plan));
    assert_eq!(stale_app.selected_occurrence_count(), 2);

    stale_app.selection.clear();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let stale_context_plan = stale_app.deselect_source_plan().unwrap();
    stale_app
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!stale_app.apply_deselect_source_plan(stale_context_plan));
    assert_eq!(stale_app.selected_occurrence_count(), 1);

    stale_app.selection.edit_context.clear();
    let mut tampered_plan = stale_app.deselect_source_plan().unwrap();
    tampered_plan.occurrence_count += 1;
    assert!(!stale_app.apply_deselect_source_plan(tampered_plan));
    assert_eq!(stale_app.selected_occurrence_count(), 1);
}

#[test]
fn select_all_plan_is_exact_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let copy_plan = app.copy_source_plan().unwrap();
    assert!(app.apply_copy_source_plan(copy_plan));
    let clipboard = app.occurrence_clipboard.clone();
    app.selection.clear();

    let plan = app.select_all_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert!(plan.source_occurrence_paths.is_empty());
    assert!(plan.source_primary.is_none());
    assert!(plan.source_selected_group.is_none());
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.target_count, 2);
    assert_eq!(
        plan.target_instance_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_select_all_source_plan(plan));

    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.occurrence_clipboard, clipboard);
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-selected-all",
            &BTreeMap::from([("count", "2".to_owned())])
        )
    );
    assert!(app.select_all_source_plan().is_none());
    assert!(!app.select_all());

    app.selection.clear();
    let stale_revision_plan = app.select_all_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_select_all_source_plan(stale_revision_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection.clear();
    let stale_selection_plan = app.select_all_source_plan().unwrap();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    assert!(!app.apply_select_all_source_plan(stale_selection_plan));
    assert_eq!(app.selected_occurrence_count(), 1);

    app.selection.clear();
    let stale_context_plan = app.select_all_source_plan().unwrap();
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_select_all_source_plan(stale_context_plan));
    assert!(app.selection.occurrences.is_empty());
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut tampered_app = KetchupApp::new();
    let mut tampered_plan = tampered_app.select_all_source_plan().unwrap();
    tampered_plan.target_count += 1;
    assert!(!tampered_app.apply_select_all_source_plan(tampered_plan));
    assert_eq!(tampered_app.selected_occurrence_count(), 0);
}

#[test]
fn select_all_plan_stays_inside_group_and_definition_contexts() {
    let mut group_app = KetchupApp::new();
    assert!(group_app.create_box());
    assert!(group_app.create_box());
    group_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    group_app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(group_app.group_selected());
    let group_id = group_app.selection.selected_group.unwrap();
    assert!(group_app.enter_group_context(group_id));
    let group_plan = group_app.select_all_source_plan().unwrap();
    assert_eq!(
        group_plan.target_instance_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    let group_revision = group_app.document_revision();
    let group_digest = group_app.canonical_digest();
    let group_undo_steps = group_app.undo_step_count();
    assert!(group_app.select_all());
    assert!(!group_app.command_enabled(AppCommand::SelectAll));
    assert_eq!(group_app.document_revision(), group_revision);
    assert_eq!(group_app.canonical_digest(), group_digest);
    assert_eq!(group_app.undo_step_count(), group_undo_steps);

    let mut definition_app = KetchupApp::new();
    definition_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(definition_app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    definition_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    definition_app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(definition_app.group_selected());
    assert!(definition_app.make_component());
    let component_path = definition_app
        .selection
        .occurrences
        .first()
        .unwrap()
        .clone();
    assert!(definition_app.enter_occurrence_context(component_path.clone()));
    let definition_plan = definition_app.select_all_source_plan().unwrap();
    assert!(definition_plan.target_instance_paths.len() >= 2);
    assert!(
        definition_plan
            .target_instance_paths
            .iter()
            .all(|path| path.root_occurrence() == component_path.root_occurrence())
    );
    assert!(definition_app.select_all());
    assert_eq!(
        definition_app.selected_instance_paths(),
        definition_plan.target_instance_paths
    );
    assert!(!definition_app.command_enabled(AppCommand::SelectAll));
}

#[test]
fn select_all_instances_stays_inside_the_active_group_context() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    assert!(app.copy_selected(Vec3::new(300.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    assert!(app.enter_group_context(group_id));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.command_enabled(AppCommand::SelectAllInstances));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    app.dispatch_command(AppCommand::SelectAllInstances);

    assert_eq!(
        app.selected_instance_paths(),
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn select_all_instances_rejects_group_and_missing_selections_without_mutation() {
    let mut group_app = KetchupApp::new();
    group_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(group_app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    let group_id = GroupId(10);
    group_app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group_id,
                name: "Single".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(group_id),
            },
        ]))
        .unwrap();
    assert!(group_app.select_group(group_id));
    let revision = group_app.document_revision();
    let digest = group_app.canonical_digest();
    let undo_steps = group_app.undo_step_count();
    let action_digest = group_app.action_digest().to_owned();

    assert!(group_app.select_all_instances_source_plan().is_none());
    assert!(!group_app.command_enabled(AppCommand::SelectAllInstances));
    assert!(!group_app.select_all_instances());
    assert_eq!(group_app.document_revision(), revision);
    assert_eq!(group_app.canonical_digest(), digest);
    assert_eq!(group_app.undo_step_count(), undo_steps);
    assert_eq!(group_app.action_digest(), action_digest);

    let mut missing_app = KetchupApp::new();
    missing_app.selection.clear();
    missing_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let action_digest = missing_app.action_digest().to_owned();
    assert!(missing_app.select_all_instances_source_plan().is_none());
    assert!(!missing_app.command_enabled(AppCommand::SelectAllInstances));
    assert!(!missing_app.select_all_instances());
    assert_eq!(missing_app.action_digest(), action_digest);
}

#[test]
fn select_all_instances_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    let plan = app.select_all_instances_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.source_instance_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );
    assert_eq!(plan.source_count, 1);
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(
        plan.target_instance_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(plan.target_count, 2);

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let selection = app.selected_instance_paths();
    let mut tampered = plan.clone();
    tampered.target_count += 1;
    assert!(!app.apply_select_all_instances_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    let context_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_select_all_instances_source_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), context_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);
    app.selection.edit_context.pop();

    assert!(app.set_selected_occurrence_grounded(true));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_select_all_instances_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn grounded_occurrence_plan_is_exact_and_rejects_tampering_context_drift_and_staleness() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    let plan = app.grounded_occurrence_source_plan(true).unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(plan.source_digest, app.canonical_digest());
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );
    assert_eq!(plan.occurrence_count, 1);
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.occurrence_id, OccurrenceId(1));
    assert!(!plan.source_grounded);
    assert!(plan.target_grounded);
    assert_eq!(
        plan.command,
        CanonicalCommand::SetOccurrenceGrounded {
            id: OccurrenceId(1),
            grounded: true,
        }
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let selection = app.selected_instance_paths();
    let mut tampered = plan.clone();
    tampered.command = CanonicalCommand::SetOccurrenceGrounded {
        id: OccurrenceId(1),
        grounded: false,
    };
    assert!(!app.apply_grounded_occurrence_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_grounded_occurrence_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.set_selected_occurrence_grounded(true));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_grounded_occurrence_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn selection_visibility_plan_is_exact_and_rejects_tampering_context_drift_and_staleness() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    let plan = app.selection_visibility_source_plan(false).unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(plan.source_digest, app.canonical_digest());
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );
    assert_eq!(plan.occurrence_count, 1);
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(
        plan.source_visibility,
        BTreeMap::from([(OccurrenceId(1), true)])
    );
    assert_eq!(
        plan.changed_occurrence_ids,
        BTreeSet::from([OccurrenceId(1)])
    );
    assert!(!plan.target_visible);
    assert_eq!(
        plan.commands,
        vec![CanonicalCommand::SetOccurrenceVisibility {
            id: OccurrenceId(1),
            visible: false,
        }]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let selection = app.selected_instance_paths();
    let mut tampered = plan.clone();
    tampered.commands = vec![CanonicalCommand::SetOccurrenceVisibility {
        id: OccurrenceId(1),
        visible: true,
    }];
    assert!(!app.apply_selection_visibility_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_selection_visibility_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.set_selected_occurrence_grounded(true));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_selection_visibility_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn tag_creation_plan_is_exact_and_rejects_tampering_namespace_selection_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let existing_tag = TagId(700);
    let created_tag = TagId(701);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: existing_tag,
            name: "Other".to_owned(),
            visible: false,
        }]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let occurrence_ids = BTreeSet::from([OccurrenceId(1)]);
    let selection = app.selected_instance_paths();
    let clipboard = app.occurrence_clipboard.clone();
    let source = app.tag_creation_source_plan(Some(&occurrence_ids)).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([(existing_tag, ("Other".to_owned(), false))])
    );
    assert_eq!(source.id, created_tag);
    assert_eq!(source.occurrence_ids, Some(occurrence_ids));
    let plan = app.tag_creation_plan(&source, "  Hardware  ").unwrap();
    assert_eq!(plan.target_name, "Hardware");
    assert_eq!(
        plan.commands,
        vec![
            CanonicalCommand::CreateTag {
                id: created_tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(created_tag),
            },
        ]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands.pop();
    assert!(!app.apply_tag_creation_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&existing_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_creation_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);

    app.clear_selection();
    assert!(!app.apply_tag_creation_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.occurrence_clipboard, clipboard);
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);

    assert!(app.apply_tag_creation_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let created = app.document_snapshot();
    assert_eq!(created.tag(created_tag).unwrap().name(), "Hardware");
    assert!(created.tag(created_tag).unwrap().visible());
    assert_eq!(
        created.occurrence(OccurrenceId(1)).unwrap().tag(),
        Some(created_tag)
    );
    assert_eq!(created.tag(existing_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_creation_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);

    assert!(app.undo());
    assert!(app.document_snapshot().tag(created_tag).is_none());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        None
    );
    assert!(app.redo());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        Some(created_tag)
    );
}

#[test]
fn tag_clear_plan_is_exact_and_rejects_tampering_namespace_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let tag = TagId(698);
    let other_tag = TagId(699);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: other_tag,
                name: "Other".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(tag),
            },
        ]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let selection = app.selected_instance_paths();
    let clipboard = app.occurrence_clipboard.clone();
    let source = app.tag_clear_source_plan(tag).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([
            (tag, ("Hardware".to_owned(), true)),
            (other_tag, ("Other".to_owned(), false)),
        ])
    );
    assert_eq!(source.id, tag);
    assert_eq!(source.original_name, "Hardware");
    assert!(source.original_visible);
    assert_eq!(source.occurrence_ids, BTreeSet::from([OccurrenceId(1)]));
    let plan = app.tag_clear_plan(&source).unwrap();
    assert_eq!(
        plan.commands,
        vec![CanonicalCommand::SetOccurrenceTag {
            id: OccurrenceId(1),
            tag: None,
        }]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands = vec![CanonicalCommand::SetOccurrenceTag {
        id: OccurrenceId(1),
        tag: Some(other_tag),
    }];
    assert!(!app.apply_tag_clear_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&other_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_clear_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.apply_tag_clear_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let cleared = app.document_snapshot();
    assert_eq!(cleared.tag(tag).unwrap().name(), "Hardware");
    assert!(cleared.tag(tag).unwrap().visible());
    assert_eq!(cleared.occurrence(OccurrenceId(1)).unwrap().tag(), None);
    assert_eq!(cleared.tag(other_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_clear_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.undo());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        Some(tag)
    );
    assert!(app.redo());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        None
    );
}

#[test]
fn tag_deletion_plan_is_exact_and_rejects_tampering_namespace_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let tag = TagId(700);
    let other_tag = TagId(701);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: other_tag,
                name: "Other".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(tag),
            },
        ]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let selection = app.selected_instance_paths();
    let clipboard = app.occurrence_clipboard.clone();
    let source = app.tag_deletion_source_plan(tag).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([
            (tag, ("Hardware".to_owned(), true)),
            (other_tag, ("Other".to_owned(), false)),
        ])
    );
    assert_eq!(source.id, tag);
    assert_eq!(source.original_name, "Hardware");
    assert!(source.original_visible);
    assert_eq!(source.occurrence_ids, BTreeSet::from([OccurrenceId(1)]));
    let plan = app.tag_deletion_plan(&source).unwrap();
    assert_eq!(
        plan.commands,
        vec![
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: None,
            },
            CanonicalCommand::DeleteTag { id: tag },
        ]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands = vec![CanonicalCommand::DeleteTag { id: tag }];
    assert!(!app.apply_tag_deletion_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&other_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_deletion_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.apply_tag_deletion_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let deleted = app.document_snapshot();
    assert!(deleted.tag(tag).is_none());
    assert_eq!(deleted.occurrence(OccurrenceId(1)).unwrap().tag(), None);
    assert_eq!(deleted.tag(other_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_deletion_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.undo());
    assert_eq!(app.document_snapshot().tag(tag).unwrap().name(), "Hardware");
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        Some(tag)
    );
    assert!(app.redo());
    assert!(app.document_snapshot().tag(tag).is_none());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tag(),
        None
    );
}

#[test]
fn tag_rename_plan_is_exact_and_rejects_tampering_namespace_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let tag = TagId(700);
    let other_tag = TagId(701);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: other_tag,
                name: "Other".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(tag),
            },
        ]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let selection = app.selected_instance_paths();
    let clipboard = app.occurrence_clipboard.clone();
    let source = app.tag_rename_source_plan(tag).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([
            (tag, ("Hardware".to_owned(), true)),
            (other_tag, ("Other".to_owned(), true)),
        ])
    );
    assert_eq!(source.id, tag);
    assert_eq!(source.original_name, "Hardware");
    assert!(source.original_visible);
    assert_eq!(source.occurrence_ids, BTreeSet::from([OccurrenceId(1)]));
    assert!(app.tag_rename_plan(&source, "Other").is_none());
    let plan = app.tag_rename_plan(&source, "  Mechanical  ").unwrap();
    assert_eq!(plan.target_name, "Mechanical");
    assert_eq!(
        plan.command,
        CanonicalCommand::SetTagName {
            id: tag,
            name: "Mechanical".to_owned(),
        }
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.command = CanonicalCommand::SetTagName {
        id: tag,
        name: "Tampered".to_owned(),
    };
    assert!(!app.apply_tag_rename_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&other_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_rename_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.apply_tag_rename_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let renamed = app.document_snapshot();
    assert_eq!(renamed.tag(tag).unwrap().name(), "Mechanical");
    assert!(renamed.tag(tag).unwrap().visible());
    assert_eq!(
        renamed.occurrence(OccurrenceId(1)).unwrap().tag(),
        Some(tag)
    );
    assert_eq!(renamed.tag(other_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_rename_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.undo());
    assert_eq!(app.document_snapshot().tag(tag).unwrap().name(), "Hardware");
    assert!(app.redo());
    assert_eq!(
        app.document_snapshot().tag(tag).unwrap().name(),
        "Mechanical"
    );
}

#[test]
fn tag_assignment_plan_is_exact_and_rejects_tampering_context_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let source_tag = TagId(700);
    let target_tag = TagId(701);
    assert!(app.create_box());
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: source_tag,
                name: "Source".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: target_tag,
                name: "Target".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(source_tag),
            },
        ]))
        .unwrap();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    let source = app.tag_assignment_source_plan().unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.occurrence_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(source.occurrence_count, 2);
    assert_eq!(source.source_primary, None);
    assert_eq!(source.source_selected_group, None);
    assert!(source.edit_context.is_empty());
    assert_eq!(
        source.source_tags,
        BTreeMap::from([(OccurrenceId(1), Some(source_tag)), (OccurrenceId(2), None),])
    );
    assert_eq!(
        source.available_tags.get(&target_tag).map(String::as_str),
        Some("Target")
    );
    assert_eq!(source.initial_tag, None);
    let pending = PendingTagAssignment {
        source,
        target_tag: Some(target_tag),
    };
    let plan = app.dialog_tag_assignment_plan(&pending).unwrap();
    assert_eq!(
        plan.changed_occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(
        plan.commands,
        vec![
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(1),
                tag: Some(target_tag),
            },
            CanonicalCommand::SetOccurrenceTag {
                id: OccurrenceId(2),
                tag: Some(target_tag),
            },
        ]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands.pop();
    assert!(!app.apply_tag_assignment_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_tag_assignment_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.apply_tag_assignment_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.occurrence_tag(OccurrenceId(1)), Some(target_tag));
    assert_eq!(app.occurrence_tag(OccurrenceId(2)), Some(target_tag));
    assert_eq!(app.occurrence_clipboard, clipboard);
    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_assignment_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.undo());
    assert_eq!(app.occurrence_tag(OccurrenceId(1)), Some(source_tag));
    assert_eq!(app.occurrence_tag(OccurrenceId(2)), None);
    assert!(app.redo());
    assert_eq!(app.occurrence_tag(OccurrenceId(1)), Some(target_tag));
    assert_eq!(app.occurrence_tag(OccurrenceId(2)), Some(target_tag));
}

#[test]
fn component_replacement_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let source = app.component_replacement_source_plan().unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(
        source.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(2))])
    );
    assert_eq!(source.occurrence_count, 1);
    assert_eq!(source.occurrence_id, OccurrenceId(2));
    assert_eq!(source.source_primary, None);
    assert_eq!(source.source_selected_group, None);
    assert!(source.edit_context.is_empty());
    assert_eq!(source.source_definition_id, DefinitionId(2));
    assert_eq!(source.candidate_count, 1);
    assert_eq!(source.initial_target_definition_id, DefinitionId(1));
    assert_eq!(
        source.candidate_definitions.get(&DefinitionId(1)),
        app.definition_name(DefinitionId(1)).as_ref()
    );
    let pending = PendingComponentReplacement {
        source,
        target_definition_id: DefinitionId(1),
    };
    let plan = app.component_replacement_plan(&pending).unwrap();
    assert_eq!(
        plan.command,
        CanonicalCommand::RepointOccurrence {
            id: OccurrenceId(2),
            definition_id: DefinitionId(1),
        }
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    let mut tampered = plan.clone();
    tampered.command = CanonicalCommand::RepointOccurrence {
        id: OccurrenceId(2),
        definition_id: DefinitionId(999),
    };
    assert!(!app.apply_component_replacement_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source.source.source_definition_name.push('!');
    assert!(!app.apply_component_replacement_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_component_replacement_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_component_replacement_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn make_unique_rejects_a_missing_selected_occurrence_without_mutation() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.make_unique_source_plan().is_none());
    assert!(!app.make_unique());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn make_unique_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.occurrence_clipboard.clone();
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let plan = app.make_unique_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(2))])
    );
    assert_eq!(plan.occurrence_id, OccurrenceId(2));
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.source_definition_id, DefinitionId(1));
    assert_eq!(plan.visible_peer_count, 2);
    assert_eq!(
        plan.visible_peer_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    let mut tampered = plan.clone();
    tampered.command = CloneDefinitionPlan::new(
        OccurrenceId(2),
        DefinitionId(1),
        DefinitionId(999),
        "Tampered".to_owned(),
        Vec::new(),
    );
    assert!(!app.apply_make_unique_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_make_unique_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);

    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_make_unique_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.occurrence_clipboard, clipboard);
}

#[test]
fn ground_occurrence_rejects_a_missing_selected_occurrence_without_mutation() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    assert!(app.grounded_occurrence_source_plan(true).is_none());
    assert!(!app.command_enabled(AppCommand::GroundOccurrence));
    assert!(!app.set_selected_occurrence_grounded(true));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
}

#[test]
fn selection_visibility_rejects_a_partly_missing_selection_without_mutation() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    assert!(app.selection_visibility_source_plan(false).is_none());
    assert!(!app.command_enabled(AppCommand::Hide));
    assert!(!app.set_selection_visibility(false));
    assert!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .visible()
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
}

#[test]
fn delete_selection_rejects_missing_and_collection_protected_occurrences() {
    let mut stale = KetchupApp::new();
    stale.selection.clear();
    stale
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    stale
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let stale_revision = stale.document_revision();
    let stale_digest = stale.canonical_digest();
    let stale_undo_steps = stale.undo_step_count();
    let stale_action_digest = stale.action_digest().to_owned();

    assert!(stale.delete_selection_source_plan().is_none());
    assert!(!stale.command_enabled(AppCommand::Delete));
    assert!(!stale.delete_selected());
    assert_eq!(stale.document_revision(), stale_revision);
    assert_eq!(stale.canonical_digest(), stale_digest);
    assert_eq!(stale.undo_step_count(), stale_undo_steps);
    assert_eq!(stale.action_digest(), stale_action_digest);

    let mut protected = KetchupApp::new();
    assert!(
        protected
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateCollection {
                    id: CollectionId(1),
                    name: "Protected".to_owned(),
                },
                CanonicalCommand::SetCollectionOccurrences {
                    id: CollectionId(1),
                    occurrence_ids: vec![OccurrenceId(1)],
                },
            ]))
            .is_ok()
    );
    protected.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let protected_revision = protected.document_revision();
    let protected_digest = protected.canonical_digest();
    let protected_undo_steps = protected.undo_step_count();
    let protected_action_digest = protected.action_digest().to_owned();

    assert!(protected.delete_selection_source_plan().is_none());
    assert!(!protected.command_enabled(AppCommand::Delete));
    assert!(!protected.delete_selected());
    assert_eq!(protected.document_revision(), protected_revision);
    assert_eq!(protected.canonical_digest(), protected_digest);
    assert_eq!(protected.undo_step_count(), protected_undo_steps);
    assert_eq!(protected.action_digest(), protected_action_digest);
    assert!(
        protected
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .is_some()
    );
}

#[test]
fn organized_component_hierarchy_round_trips_with_stable_identity() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: DefinitionId(1),
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    assert!(app.enter_group_context(group_id));
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    assert!(app.make_unique());

    let expected = app.document.current();
    let loaded = ketchup_core::persistence::load(&ketchup_core::persistence::save(&expected))
        .unwrap()
        .snapshot();
    assert_eq!(loaded.canonical_digest(), expected.canonical_digest());
    assert_eq!(
        loaded.occurrence(OccurrenceId(1)).unwrap().parent(),
        Some(group_id)
    );
    assert_eq!(
        loaded.occurrence(OccurrenceId(2)).unwrap().parent(),
        Some(group_id)
    );
    assert_ne!(
        loaded.occurrence(OccurrenceId(1)).unwrap().definition_id(),
        loaded.occurrence(OccurrenceId(2)).unwrap().definition_id()
    );
}

#[test]
fn review_only_open_preserves_the_active_document_and_its_history() {
    let directory = tempfile::tempdir().unwrap();
    let active_path = directory.path().join("active.ketchup");
    let review_path = directory.path().join("legacy.ketchup");
    std::fs::write(&review_path, lossy_legacy_document()).unwrap();

    let mut app = KetchupApp::new();
    assert!(app.save_document_to(&active_path));
    let node_id = ketchup_core::document::NodeId(900);
    let active_revision = app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: node_id,
                name: "active parameter".to_owned(),
                dimension: Dimension::new("2", 2.0).unwrap(),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: node_id,
                dimension: Dimension::new("3", 3.0).unwrap(),
            },
        ]))
        .unwrap();
    assert!(app.undo());
    assert_eq!(app.document.visible_undo_steps(), 1);
    assert_eq!(app.document.visible_redo_steps(), 1);

    let before = app.document.current();
    assert_eq!(
        before.canonical_digest(),
        active_revision.snapshot().canonical_digest()
    );
    let before_document_id = before.document_id();
    let before_revision = before.revision_id();
    let before_digest = before.canonical_digest();
    let before_canonical_bytes = ketchup_core::persistence::save(&before);
    let before_evaluation = before.evaluate(&Default::default()).unwrap();
    let before_path = app.document_path.clone();
    let before_dirty = app.is_dirty();
    let before_undo_steps = app.document.visible_undo_steps();
    let before_redo_steps = app.document.visible_redo_steps();
    let before_revision_count = app.document.revision_count();
    let before_evaluation_registry = app.document.evaluation_registry_len();

    assert!(!app.open_document_from(&review_path));
    assert!(app.has_review_candidate());
    assert!(!app.review_candidate.as_ref().unwrap().is_editable());

    let after = app.document.current();
    assert_eq!(after.document_id(), before_document_id);
    assert_eq!(after.revision_id(), before_revision);
    assert_eq!(after.canonical_digest(), before_digest);
    assert_eq!(
        ketchup_core::persistence::save(&after),
        before_canonical_bytes
    );
    assert_eq!(
        after.evaluate(&Default::default()).unwrap(),
        before_evaluation
    );
    assert_eq!(app.document_path, before_path);
    assert_eq!(app.is_dirty(), before_dirty);
    assert_eq!(app.document.visible_undo_steps(), before_undo_steps);
    assert_eq!(app.document.visible_redo_steps(), before_redo_steps);
    assert_eq!(app.document.revision_count(), before_revision_count);
    assert_eq!(
        app.document.evaluation_registry_len(),
        before_evaluation_registry
    );
}

#[test]
fn migration_confirmation_rejects_review_candidate_tamper_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy-review.ketchup");
    let destination = directory.path().join("tampered-review-migration.ketchup");
    let source_bytes = lossy_legacy_document();
    std::fs::write(&source, &source_bytes).unwrap();

    let mut app = KetchupApp::new();
    let before = app.document.current();
    assert!(!app.open_document_from(&source));
    assert!(app.has_review_candidate());

    let mut alternate_source = source_bytes;
    let value_start = alternate_source.len() - 12;
    alternate_source[value_start..value_start + 8]
        .copy_from_slice(&4.5_f64.to_bits().to_le_bytes());
    app.review_candidate = Some(ketchup_core::persistence::load(&alternate_source).unwrap());

    assert!(!app.confirm_review_candidate_migration_to(&destination));
    assert!(!destination.exists());
    assert!(app.has_review_candidate());
    let after = app.document.current();
    assert_eq!(after.document_id(), before.document_id());
    assert_eq!(after.revision_id(), before.revision_id());
    assert_eq!(after.canonical_digest(), before.canonical_digest());
}

#[test]
fn lossless_schema_three_open_replaces_the_document_and_clears_history_and_review() {
    let directory = tempfile::tempdir().unwrap();
    let review_path = directory.path().join("legacy.ketchup");
    let lossless_path = directory.path().join("lossless.ketchup");
    std::fs::write(&review_path, lossy_legacy_document()).unwrap();

    let mut source = KetchupApp::new();
    assert!(source.create_box());
    assert!(source.create_box());
    let expected = source.document.current();
    let expected_bytes = ketchup_core::persistence::save(&expected);
    assert!(source.save_document_to(&lossless_path));

    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let replaced_document_id = app.document.current().document_id();
    assert!(app.document.visible_undo_steps() > 0);
    assert!(!app.open_document_from(&review_path));
    assert!(app.has_review_candidate());

    assert!(app.open_document_from(&lossless_path));

    let opened = app.document.current();
    assert_ne!(opened.document_id(), replaced_document_id);
    assert_eq!(opened.document_id(), expected.document_id());
    assert_eq!(opened.revision_id(), expected.revision_id());
    assert_eq!(opened.canonical_digest(), expected.canonical_digest());
    assert_eq!(ketchup_core::persistence::save(&opened), expected_bytes);
    assert_eq!(app.document_path.as_deref(), Some(lossless_path.as_path()));
    assert!(!app.is_dirty());
    assert_eq!(app.document.visible_undo_steps(), 0);
    assert_eq!(app.document.visible_redo_steps(), 0);
    assert_eq!(app.document.revision_count(), 1);
    assert!(!app.has_review_candidate());
}

#[test]
fn file_workflow_round_trips_composed_model_and_tracks_dirty_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("composed.ketchup");
    let mut app = KetchupApp::new();
    assert!(!app.is_dirty());

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let expected = app.document.current();
    assert!(app.is_dirty());
    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());
    assert_eq!(app.document_path.as_deref(), Some(path.as_path()));

    let mut reopened = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(1),
    ));
    assert!(reopened.open_document_from(&path));
    let actual = reopened.document.current();
    assert_eq!(actual.canonical_digest(), expected.canonical_digest());
    assert_eq!(actual.revision_id(), expected.revision_id());
    assert_eq!(actual.document_id(), expected.document_id());
    assert_eq!(actual.units(), expected.units());
    assert_eq!(actual.definitions().count(), 1);
    assert_eq!(actual.occurrences().count(), 2);
    assert_eq!(actual.groups().count(), 1);
    assert_eq!(actual.scene_query()[0].shared_occurrence_count, 2);
    assert_eq!(
        actual.occurrence(OccurrenceId(1)).unwrap().parent(),
        actual.occurrence(OccurrenceId(2)).unwrap().parent()
    );
    assert_eq!(
        actual.occurrence(OccurrenceId(2)).unwrap().transform(),
        expected.occurrence(OccurrenceId(2)).unwrap().transform()
    );
    assert_eq!(
        actual.feature(FeatureId(2)).unwrap().kind(),
        expected.feature(FeatureId(2)).unwrap().kind()
    );
    assert!(!reopened.is_dirty());
    assert_eq!(reopened.document.visible_undo_steps(), 0);

    reopened.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(reopened.move_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert!(reopened.is_dirty());
    assert!(reopened.undo());
    assert!(!reopened.is_dirty());
    assert!(reopened.redo());
    assert!(reopened.is_dirty());
    assert!(reopened.save_document_to(&path));
    let saved_again = ketchup_core::persistence::load_file(&path)
        .unwrap()
        .snapshot();
    assert_eq!(
        saved_again.canonical_digest(),
        reopened.document.current().canonical_digest()
    );
}

#[test]
fn failed_open_and_save_preserve_the_active_document_and_file_identity() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.ketchup");
    std::fs::write(&malformed, b"not a ketchup document").unwrap();
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let before_digest = app.document.current().canonical_digest();
    let before_path = app.document_path.clone();
    let before_saved_digest = app.saved_digest.clone();

    assert!(!app.open_document_from(&malformed));
    assert_eq!(app.document.current().canonical_digest(), before_digest);
    assert_eq!(app.document_path, before_path);
    assert_eq!(app.saved_digest, before_saved_digest);
    assert!(app.digest.contains("active model was not changed"));

    assert!(!app.save_document_to(directory.path()));
    assert_eq!(app.document.current().canonical_digest(), before_digest);
    assert_eq!(app.document_path, before_path);
    assert_eq!(app.saved_digest, before_saved_digest);
    assert!(app.is_dirty());
    assert!(app.digest.contains("active model remains unsaved"));
}

#[test]
fn command_registry_exposes_only_complete_modeling_tools() {
    let mut app = KetchupApp::new();
    assert!(app.command_enabled(AppCommand::Select));
    assert!(app.command_enabled(AppCommand::Orbit));
    assert!(app.command_enabled(AppCommand::Rectangle));
    assert!(app.command_enabled(AppCommand::PushPull));
    assert!(app.command_enabled(AppCommand::Move));

    app.dispatch_command(AppCommand::Rectangle);
    assert_eq!(app.active_tool, ActiveTool::Rectangle);
    assert!(app.sketch_mode);
    app.dispatch_command(AppCommand::PushPull);
    assert_eq!(app.active_tool, ActiveTool::PushPull);
    assert!(!app.sketch_mode);
}
