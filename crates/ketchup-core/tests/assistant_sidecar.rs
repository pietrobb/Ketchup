use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantBalloonTextIntent, AssistantBeamNotchIntent,
    AssistantBottleFinishKind, AssistantBottleIntent, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadPartFeature, AssistantCadRotation, AssistantDistribution, AssistantHandshake,
    AssistantHandshakeError, AssistantKetchupBottleIntent, AssistantLinearArrayIntent,
    AssistantModelIntent, AssistantOrientedBeamIntent, AssistantParameterEditIntent,
    AssistantPrincipalPlane, AssistantProfileTranslationIntent, AssistantRejectionDiagnostic,
    AssistantRejectionPhase, AssistantRotationIntent, AssistantSketchConstraint,
    AssistantSketchEntity, AssistantSketchPointKind, AssistantSketchPointRef,
    AssistantTeapotIntent, AssistantWorkplaneSpec, distribution_is_enabled,
};

const PUBLIC_HANDSHAKE: &str = r#"{
    "protocol_version": 3,
    "distribution": "public-api",
    "provider": "anthropic-api",
    "model": "claude-sonnet-4-6",
    "capabilities": ["chat", "local_memory", "query_document", "propose_workflow_intent"]
}"#;

#[test]
fn public_api_handshake_accepts_only_the_bounded_contract() {
    let handshake = AssistantHandshake::parse_and_validate(PUBLIC_HANDSHAKE).unwrap();

    assert_eq!(handshake.protocol_version, ASSISTANT_PROTOCOL_VERSION);
    assert_eq!(handshake.distribution, AssistantDistribution::PublicApi);
    assert_eq!(handshake.provider, "anthropic-api");
    assert_eq!(handshake.model, "claude-sonnet-4-6");
}

#[test]
fn handshake_rejects_secrets_and_unknown_capabilities() {
    let secret = PUBLIC_HANDSHAKE.replace(
        "\n}",
        ",\n    \"api_key\": \"must-not-cross-the-protocol\"\n}",
    );
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&secret),
        Err(AssistantHandshakeError::InvalidJson(_))
    ));

    let shell = PUBLIC_HANDSHAKE.replace(
        "\"propose_workflow_intent\"",
        "\"propose_workflow_intent\", \"shell\"",
    );
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&shell),
        Err(AssistantHandshakeError::InvalidJson(_))
    ));
}

#[test]
fn handshake_rejects_unknown_providers_models_and_protocol_versions() {
    let provider = PUBLIC_HANDSHAKE.replace("anthropic-api", "arbitrary-network-provider");
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&provider),
        Err(AssistantHandshakeError::UnsupportedProvider(provider))
            if provider == "arbitrary-network-provider"
    ));

    let model = PUBLIC_HANDSHAKE.replace("claude-sonnet-4-6", "../untrusted model");
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&model),
        Err(AssistantHandshakeError::UnsupportedModel(model))
            if model == "../untrusted model"
    ));

    let version = PUBLIC_HANDSHAKE.replace("\"protocol_version\": 3", "\"protocol_version\": 4");
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&version),
        Err(AssistantHandshakeError::UnsupportedProtocolVersion(4))
    ));
}

#[test]
fn cad_edit_program_contract_is_typed_strict_and_round_trips() {
    let explicit = AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![7, 9],
    };
    let program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::Delete {
                selector: explicit.clone(),
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            },
            AssistantCadEditOperation::Transform {
                selector: AssistantCadEntitySelector::CurrentSelection {},
                translation_mm: [10.0, -2.0, 5.0],
                rotation: Some(AssistantCadRotation {
                    pivot_mm: [1.0, 2.0, 3.0],
                    axis: [1.0, 1.0, 0.0],
                    angle_degrees: 45.0,
                }),
            },
            AssistantCadEditOperation::Copy {
                selector: explicit.clone(),
                translation_mm: [25.0, 0.0, 0.0],
            },
            AssistantCadEditOperation::LinearPattern {
                selector: explicit.clone(),
                instances: 4,
                step_mm: [0.0, 20.0, 0.0],
            },
            AssistantCadEditOperation::Mirror {
                selector: explicit,
                plane_origin_mm: [0.0, 0.0, 0.0],
                plane_normal: [1.0, 0.0, 0.0],
            },
        ],
    };

    assert_eq!(program.validate(), Ok(()));
    let serialized = serde_json::to_value(&program).unwrap();
    assert_eq!(serialized["operations"][0]["operation"], "delete");
    assert_eq!(
        serialized["operations"][0]["dependency_policy"],
        "remove_references"
    );
    assert_eq!(
        serialized["operations"][1]["selector"]["type"],
        "current_selection"
    );
    assert_eq!(serialized["operations"][3]["operation"], "linear_pattern");
    assert_eq!(serialized["operations"][4]["operation"], "mirror");
    assert_eq!(
        serde_json::from_value::<AssistantCadEditProgram>(serialized).unwrap(),
        program
    );

    let unknown = serde_json::json!({
        "operations": [{
            "operation": "copy",
            "selector": {"type": "occurrences", "occurrence_ids": [7]},
            "translation_mm": [1.0, 0.0, 0.0],
            "shell_command": "bypass"
        }]
    });
    assert!(serde_json::from_value::<AssistantCadEditProgram>(unknown).is_err());
    let polluted_selection = serde_json::json!({
        "operations": [{
            "operation": "delete",
            "selector": {"type": "current_selection", "occurrence_ids": [7]},
            "dependency_policy": "reject_if_referenced"
        }]
    });
    assert!(serde_json::from_value::<AssistantCadEditProgram>(polluted_selection).is_err());
}

#[test]
fn cad_edit_sketch_contract_is_typed_strict_and_round_trips() {
    let program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::CreateSketch {
                definition_id: 1,
                name: "Mixed sketch".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities: vec![
                    AssistantSketchEntity::Line {
                        id: 1,
                        start_mm: [-10.0, 0.0],
                        end_mm: [10.0, 0.0],
                    },
                    AssistantSketchEntity::Arc {
                        id: 2,
                        start_mm: [10.0, 0.0],
                        end_mm: [-10.0, 0.0],
                        center_mm: [0.0, 0.0],
                        clockwise: false,
                    },
                    AssistantSketchEntity::Circle {
                        id: 3,
                        center_mm: [20.0, 0.0],
                        radius_mm: 5.0,
                    },
                ],
                constraints: vec![
                    AssistantSketchConstraint::Horizontal {
                        id: 1,
                        entity_id: 1,
                    },
                    AssistantSketchConstraint::Radius {
                        id: 2,
                        entity_id: 3,
                        value_mm: 5.0,
                    },
                    AssistantSketchConstraint::FixedPoint {
                        id: 3,
                        point: AssistantSketchPointRef {
                            entity_id: 1,
                            point: AssistantSketchPointKind::Start,
                        },
                        position_mm: [-10.0, 0.0],
                    },
                ],
            },
            AssistantCadEditOperation::SetDimension {
                feature_id: 12,
                constraint_id: Some(2),
                value_mm: 7.5,
            },
        ],
    };

    assert_eq!(program.validate(), Ok(()));
    let serialized = serde_json::to_value(&program).unwrap();
    assert_eq!(serialized["operations"][0]["operation"], "create_sketch");
    assert_eq!(serialized["operations"][0]["workplane"]["plane"], "xy");
    assert_eq!(serialized["operations"][0]["entities"][1]["type"], "arc");
    assert_eq!(serialized["operations"][1]["operation"], "set_dimension");
    assert_eq!(
        serde_json::from_value::<AssistantCadEditProgram>(serialized).unwrap(),
        program
    );

    let unknown_entity_field = serde_json::json!({
        "operations": [{
            "operation": "create_sketch",
            "definition_id": 1,
            "name": "Rejected",
            "workplane": {"type": "principal", "plane": "xy"},
            "entities": [{
                "type": "circle",
                "id": 1,
                "center_mm": [0.0, 0.0],
                "radius_mm": 5.0,
                "script": "bypass"
            }],
            "constraints": []
        }]
    });
    assert!(serde_json::from_value::<AssistantCadEditProgram>(unknown_entity_field).is_err());
}

#[test]
fn cad_edit_full_sketch_constraint_vocabulary_is_typed_and_round_trips() {
    let point = |entity_id, point| AssistantSketchPointRef { entity_id, point };
    let constraints = vec![
        AssistantSketchConstraint::Parallel {
            id: 1,
            a_entity_id: 1,
            b_entity_id: 2,
        },
        AssistantSketchConstraint::Perpendicular {
            id: 2,
            a_entity_id: 3,
            b_entity_id: 4,
        },
        AssistantSketchConstraint::Tangent {
            id: 3,
            a_entity_id: 5,
            b_entity_id: 6,
        },
        AssistantSketchConstraint::Angle {
            id: 4,
            a_entity_id: 7,
            b_entity_id: 8,
            angle_degrees: 60.0,
        },
        AssistantSketchConstraint::Equal {
            id: 5,
            a_entity_id: 9,
            b_entity_id: 10,
        },
        AssistantSketchConstraint::Symmetric {
            id: 6,
            a: point(12, AssistantSketchPointKind::Start),
            b: point(13, AssistantSketchPointKind::Start),
            axis_entity_id: 11,
        },
        AssistantSketchConstraint::Concentric {
            id: 7,
            a_entity_id: 14,
            b_entity_id: 15,
        },
        AssistantSketchConstraint::Collinear {
            id: 8,
            a_entity_id: 16,
            b_entity_id: 17,
        },
        AssistantSketchConstraint::Midpoint {
            id: 9,
            point: point(19, AssistantSketchPointKind::Start),
            line_entity_id: 18,
        },
        AssistantSketchConstraint::PointOnCurve {
            id: 10,
            point: point(21, AssistantSketchPointKind::Start),
            curve_entity_id: 20,
        },
    ];
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreateSketch {
            definition_id: 1,
            name: "General constraints".to_owned(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: (1..=21)
                .map(|id| {
                    if [6, 14, 15, 20].contains(&id) {
                        AssistantSketchEntity::Circle {
                            id,
                            center_mm: [id as f64, 0.0],
                            radius_mm: 1.0,
                        }
                    } else {
                        AssistantSketchEntity::Line {
                            id,
                            start_mm: [id as f64, 0.0],
                            end_mm: [id as f64, 1.0],
                        }
                    }
                })
                .collect(),
            constraints,
        }],
    };

    assert_eq!(program.validate(), Ok(()));
    let serialized = serde_json::to_value(&program).unwrap();
    let constraint_json = serialized["operations"][0]["constraints"]
        .as_array()
        .unwrap();
    assert_eq!(constraint_json[0]["type"], "parallel");
    assert_eq!(constraint_json[9]["type"], "point_on_curve");
    assert_eq!(
        serde_json::from_value::<AssistantCadEditProgram>(serialized).unwrap(),
        program
    );

    let mut invalid_angle = program.clone();
    let AssistantCadEditOperation::CreateSketch { constraints, .. } =
        &mut invalid_angle.operations[0]
    else {
        unreachable!()
    };
    let AssistantSketchConstraint::Angle { angle_degrees, .. } = &mut constraints[3] else {
        unreachable!()
    };
    *angle_degrees = 180.0;
    assert!(invalid_angle.validate().is_err());

    let mut invalid_tangent = program.clone();
    let AssistantCadEditOperation::CreateSketch { constraints, .. } =
        &mut invalid_tangent.operations[0]
    else {
        unreachable!()
    };
    let AssistantSketchConstraint::Tangent { b_entity_id, .. } = &mut constraints[2] else {
        unreachable!()
    };
    *b_entity_id = 4;
    assert!(invalid_tangent.validate().is_err());

    let mut missing_reference = program;
    let AssistantCadEditOperation::CreateSketch { constraints, .. } =
        &mut missing_reference.operations[0]
    else {
        unreachable!()
    };
    let AssistantSketchConstraint::Midpoint { line_entity_id, .. } = &mut constraints[8] else {
        unreachable!()
    };
    *line_entity_id = 999;
    assert!(missing_reference.validate().is_err());
}

#[test]
fn cad_edit_part_contract_is_typed_bounded_and_round_trips() {
    let program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Editable prism".to_owned(),
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
            rotation: Some(AssistantCadRotation {
                pivot_mm: [5.0, 6.0, 7.0],
                axis: [0.0, 1.0, 0.0],
                angle_degrees: 45.0,
            }),
        }],
    };

    assert_eq!(program.validate(), Ok(()));
    let serialized = serde_json::to_value(&program).unwrap();
    assert_eq!(serialized["operations"][0]["operation"], "create_part");
    assert_eq!(serialized["operations"][0]["feature"]["type"], "extrusion");
    assert_eq!(
        serde_json::from_value::<AssistantCadEditProgram>(serialized).unwrap(),
        program
    );

    let mut revolve = program.clone();
    let AssistantCadEditOperation::CreatePart { feature, .. } = &mut revolve.operations[0] else {
        unreachable!()
    };
    *feature = AssistantCadPartFeature::Revolve {
        axis_start_mm: [0.0, 0.0],
        axis_end_mm: [0.0, 1.0],
        angle_degrees: 275.0,
    };
    assert_eq!(revolve.validate(), Ok(()));
    let serialized = serde_json::to_value(&revolve).unwrap();
    assert_eq!(serialized["operations"][0]["feature"]["type"], "revolve");
    assert_eq!(
        serde_json::from_value::<AssistantCadEditProgram>(serialized).unwrap(),
        revolve
    );

    let mut invalid_feature = program.clone();
    let AssistantCadEditOperation::CreatePart { feature, .. } = &mut invalid_feature.operations[0]
    else {
        unreachable!()
    };
    *feature = AssistantCadPartFeature::Extrusion { distance_mm: 0.0 };
    assert_eq!(
        invalid_feature.validate(),
        Err("assistant CAD part feature is invalid".to_owned())
    );

    let mut invalid_revolve = program;
    let AssistantCadEditOperation::CreatePart { feature, .. } = &mut invalid_revolve.operations[0]
    else {
        unreachable!()
    };
    *feature = AssistantCadPartFeature::Revolve {
        axis_start_mm: [1.0, 1.0],
        axis_end_mm: [1.0, 1.0],
        angle_degrees: 361.0,
    };
    assert_eq!(
        invalid_revolve.validate(),
        Err("assistant CAD part feature is invalid".to_owned())
    );
}

#[test]
fn cad_edit_program_contract_fails_closed_on_targets_geometry_and_resources() {
    let copy = |occurrence_ids| AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Copy {
            selector: AssistantCadEntitySelector::Occurrences { occurrence_ids },
            translation_mm: [1.0, 0.0, 0.0],
        }],
    };
    assert!(copy(Vec::new()).validate().is_err());
    assert!(copy(vec![7, 7]).validate().is_err());
    assert!(copy(vec![0]).validate().is_err());

    let invalid_rotation = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Transform {
            selector: AssistantCadEntitySelector::CurrentSelection {},
            translation_mm: [0.0; 3],
            rotation: Some(AssistantCadRotation {
                pivot_mm: [0.0; 3],
                axis: [0.0; 3],
                angle_degrees: 90.0,
            }),
        }],
    };
    assert!(invalid_rotation.validate().is_err());

    let unbounded_mirror = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Mirror {
            selector: AssistantCadEntitySelector::CurrentSelection {},
            plane_origin_mm: [f64::INFINITY, 0.0, 0.0],
            plane_normal: [1.0, 0.0, 0.0],
        }],
    };
    assert!(unbounded_mirror.validate().is_err());

    let too_many_outputs = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::LinearPattern {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: (1..=100).collect(),
            },
            instances: 7,
            step_mm: [10.0, 0.0, 0.0],
        }],
    };
    assert_eq!(
        too_many_outputs.validate(),
        Err("assistant CAD edit program creates too many occurrences".to_owned())
    );

    let too_many_operations = AssistantCadEditProgram {
        operations: (0..65)
            .map(|_| AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::CurrentSelection {},
                dependency_policy: AssistantCadDeletePolicy::RejectIfReferenced,
            })
            .collect(),
    };
    assert!(too_many_operations.validate().is_err());
}

#[test]
fn cad_edit_program_conservatively_budgets_current_selection_outputs() {
    let selector = AssistantCadEntitySelector::CurrentSelection {};
    let mut operations = vec![AssistantCadEditOperation::LinearPattern {
        selector: selector.clone(),
        instances: 4,
        step_mm: [1.0, 0.0, 0.0],
    }];
    operations.push(AssistantCadEditOperation::Copy {
        selector: selector.clone(),
        translation_mm: [1.0, 0.0, 0.0],
    });
    operations.push(AssistantCadEditOperation::Mirror {
        selector: selector.clone(),
        plane_origin_mm: [0.0; 3],
        plane_normal: [1.0, 0.0, 0.0],
    });
    assert_eq!(
        AssistantCadEditProgram {
            operations: operations.clone(),
        }
        .validate(),
        Ok(())
    );

    operations.push(AssistantCadEditOperation::Copy {
        selector,
        translation_mm: [0.0, 1.0, 0.0],
    });
    assert_eq!(
        AssistantCadEditProgram { operations }.validate(),
        Err("assistant CAD edit program creates too many occurrences".to_owned())
    );
}

#[test]
fn cad_edit_selector_and_generated_output_boundaries_fail_closed() {
    let current = AssistantCadEntitySelector::CurrentSelection {};
    assert!(current.validate_resolved_target_count(0).is_err());
    assert_eq!(current.validate_resolved_target_count(100), Ok(()));
    assert!(current.validate_resolved_target_count(101).is_err());

    let explicit_limit = AssistantCadEntitySelector::Occurrences {
        occurrence_ids: (1..=100).collect(),
    };
    assert_eq!(explicit_limit.validate_resolved_target_count(100), Ok(()));
    let too_many_explicit = AssistantCadEntitySelector::Occurrences {
        occurrence_ids: (1..=101).collect(),
    };
    assert!(
        too_many_explicit
            .validate_resolved_target_count(100)
            .is_err()
    );

    let mut boundary_operations = vec![AssistantCadEditOperation::LinearPattern {
        selector: explicit_limit,
        instances: 6,
        step_mm: [1.0, 0.0, 0.0],
    }];
    boundary_operations.push(AssistantCadEditOperation::Copy {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: (101..=112).collect(),
        },
        translation_mm: [0.0, 1.0, 0.0],
    });
    assert_eq!(
        AssistantCadEditProgram {
            operations: boundary_operations.clone(),
        }
        .validate(),
        Ok(())
    );

    boundary_operations.push(AssistantCadEditOperation::Mirror {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![113],
        },
        plane_origin_mm: [0.0; 3],
        plane_normal: [1.0, 0.0, 0.0],
    });
    assert_eq!(
        AssistantCadEditProgram {
            operations: boundary_operations,
        }
        .validate(),
        Err("assistant CAD edit program creates too many occurrences".to_owned())
    );
}

#[test]
fn assistant_rotation_is_shape_independent_arbitrary_axis_and_fail_closed() {
    let valid = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
        rotations: vec![AssistantRotationIntent {
            occurrence_id: Some(7),
            group_id: None,
            pivot_mm: [12.5, -3.0, 40.0],
            axis: [1.0, 2.0, 3.0],
            angle_degrees: 37.25,
        }],
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    };
    assert!(valid.validate().is_ok());
    let serialized = serde_json::to_value(&valid).unwrap();
    assert_eq!(
        serialized["rotations"][0]["axis"],
        serde_json::json!([1.0, 2.0, 3.0])
    );
    assert!(serialized["rotations"][0].get("group_id").is_none());

    let zero_axis = AssistantModelIntent {
        rotations: vec![AssistantRotationIntent {
            axis: [0.0, 0.0, 0.0],
            ..valid.rotations[0].clone()
        }],
        ..valid.clone()
    };
    assert_eq!(
        zero_axis.validate(),
        Err("assistant rotation is invalid".to_owned())
    );

    let ambiguous_target = AssistantModelIntent {
        rotations: vec![AssistantRotationIntent {
            group_id: Some(8),
            ..valid.rotations[0].clone()
        }],
        ..valid.clone()
    };
    assert_eq!(
        ambiguous_target.validate(),
        Err("assistant rotation is invalid".to_owned())
    );

    let conflicting_move = AssistantModelIntent {
        translations: vec![
            ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                occurrence_id: 7,
                delta_mm: [1.0, 0.0, 0.0],
            },
        ],
        ..valid
    };
    assert_eq!(
        conflicting_move.validate(),
        Err("assistant rotation is invalid".to_owned())
    );
}

#[test]
fn assistant_array_budget_matches_the_canonical_proposal_command_limit() {
    let valid = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: vec![AssistantLinearArrayIntent {
            occurrence_ids: (1..=100).collect(),
            instances: 6,
            step_mm: [0.0, 0.0, 280.0],
        }],
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    };
    assert!(valid.validate().is_ok());

    let too_large = AssistantModelIntent {
        linear_arrays: vec![AssistantLinearArrayIntent {
            occurrence_ids: (1..=100).collect(),
            instances: 7,
            step_mm: [0.0, 0.0, 280.0],
        }],
        ..valid
    };
    assert_eq!(
        too_large.validate(),
        Err("assistant proposal creates too many array occurrences".to_owned())
    );
}

#[test]
fn assistant_profile_translation_is_single_bounded_and_unmixed() {
    let valid = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
        rotations: Vec::new(),
        profile_translations: vec![AssistantProfileTranslationIntent {
            definition_id: 1,
            body_id: 2,
            profile_id: 14,
            delta_mm: [2.0, 3.0],
        }],
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    };
    assert!(valid.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&valid).unwrap()["profile_translations"][0]["delta_mm"],
        serde_json::json!([2.0, 3.0])
    );

    let zero = AssistantModelIntent {
        rotations: Vec::new(),
        profile_translations: vec![AssistantProfileTranslationIntent {
            delta_mm: [0.0, 0.0],
            ..valid.profile_translations[0].clone()
        }],
        ..valid.clone()
    };
    assert_eq!(
        zero.validate(),
        Err("assistant profile translation is invalid".to_owned())
    );

    let mixed = AssistantModelIntent {
        translations: vec![
            ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                occurrence_id: 1,
                delta_mm: [1.0, 0.0, 0.0],
            },
        ],
        ..valid
    };
    assert_eq!(
        mixed.validate(),
        Err("assistant profile translation cannot mix geometry mutations".to_owned())
    );
}

#[test]
fn assistant_parameter_edit_is_single_bounded_and_unmixed() {
    let valid = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: vec![AssistantParameterEditIntent {
            definition_id: 1,
            body_id: 2,
            feature_id: 14,
            constraint_id: Some(3),
            value_mm: 8.5,
        }],
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    };
    assert!(valid.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&valid).unwrap()["parameter_edits"][0]["value_mm"],
        8.5
    );

    let invalid = AssistantModelIntent {
        parameter_edits: vec![AssistantParameterEditIntent {
            value_mm: 0.0,
            ..valid.parameter_edits[0].clone()
        }],
        ..valid.clone()
    };
    assert_eq!(
        invalid.validate(),
        Err("assistant parameter edit is invalid".to_owned())
    );

    let mixed = AssistantModelIntent {
        translations: vec![
            ketchup_core::assistant_sidecar::AssistantTranslationIntent {
                occurrence_id: 1,
                delta_mm: [1.0, 0.0, 0.0],
            },
        ],
        ..valid
    };
    assert_eq!(
        mixed.validate(),
        Err("assistant parameter edit cannot mix geometry mutations".to_owned())
    );
}

#[test]
fn assistant_oriented_beam_intent_is_bounded_and_rejects_invalid_notches() {
    let intent: AssistantModelIntent = serde_json::from_str(
        r#"{
            "replace_scene": false,
            "boxes": [],
            "oriented_beams": [{
                "name": "Rafter",
                "start_mm": [0.0, -483.05, 3044.07],
                "end_mm": [0.0, 1900.0, 4800.0],
                "up_hint": [0.0, 0.0, 1.0],
                "width_mm": 100.0,
                "depth_mm": 180.0,
                "bottom_notches": [{
                    "from_start_mm": 600.0,
                    "length_mm": 160.0,
                    "depth_mm": 50.0
                }]
            }]
        }"#,
    )
    .unwrap();
    assert!(intent.validate().is_ok());
    assert_eq!(intent.oriented_beams[0].width_mm, 100.0);

    let parallel_up = AssistantModelIntent {
        oriented_beams: vec![AssistantOrientedBeamIntent {
            up_hint: [0.0, 2_383.05, 1_755.93],
            ..intent.oriented_beams[0].clone()
        }],
        ..intent.clone()
    };
    assert_eq!(
        parallel_up.validate(),
        Err("assistant oriented beam axis or up hint is invalid".to_owned())
    );

    let overlapping = AssistantModelIntent {
        oriented_beams: vec![AssistantOrientedBeamIntent {
            bottom_notches: vec![
                AssistantBeamNotchIntent {
                    from_start_mm: 600.0,
                    length_mm: 160.0,
                    depth_mm: 50.0,
                },
                AssistantBeamNotchIntent {
                    from_start_mm: 700.0,
                    length_mm: 100.0,
                    depth_mm: 40.0,
                },
            ],
            ..intent.oriented_beams[0].clone()
        }],
        ..intent
    };
    assert_eq!(
        overlapping.validate(),
        Err("assistant oriented beam notches overlap".to_owned())
    );
}

#[test]
fn assistant_bottle_intent_round_trips_and_reuses_exact_geometry_limits() {
    let intent: AssistantModelIntent = serde_json::from_str(
        r#"{
            "replace_scene": false,
            "boxes": [],
            "bottles": [{
                "name": "AI ketchup bottle",
                "body_radius_mm": 30.0,
                "body_height_mm": 110.0,
                "shoulder_rise_mm": 20.0,
                "neck_radius_mm": 12.0,
                "neck_height_mm": 25.0,
                "wall_thickness_mm": 2.0,
                "finish_kind": "fillet",
                "finish_amount_mm": 2.0,
                "origin_mm": [90.0, 0.0, 0.0]
            }]
        }"#,
    )
    .unwrap();

    assert!(intent.validate().is_ok());
    assert_eq!(
        intent.bottles[0].finish_kind,
        AssistantBottleFinishKind::Fillet
    );
    let encoded = serde_json::to_value(&intent).unwrap();
    assert_eq!(encoded["bottles"][0]["neck_height_mm"], 25.0);
    assert_eq!(encoded["bottles"][0]["finish_kind"], "fillet");

    let invalid = AssistantModelIntent {
        bottles: vec![AssistantBottleIntent {
            wall_thickness_mm: 6.0,
            ..intent.bottles[0].clone()
        }],
        ..intent
    };
    assert_eq!(
        invalid.validate(),
        Err("assistant bottle wall thickness is unsupported".to_owned())
    );
}

#[test]
fn assistant_teapot_intent_round_trips_and_rejects_impossible_attachments() {
    let intent: AssistantModelIntent = serde_json::from_str(
        r#"{
            "replace_scene": false,
            "boxes": [],
            "bottles": [{
                "name": "Rounded tea pot",
                "body_radius_mm": 70.0,
                "body_height_mm": 105.0,
                "shoulder_rise_mm": 22.0,
                "neck_radius_mm": 42.0,
                "neck_height_mm": 14.0,
                "wall_thickness_mm": 3.0,
                "finish_kind": "fillet",
                "finish_amount_mm": 4.0,
                "origin_mm": [0.0, 0.0, 0.0],
                "teapot": {
                    "handle_clearance_mm": 52.0,
                    "handle_tube_radius_mm": 9.0,
                    "spout_length_mm": 105.0,
                    "spout_radius_mm": 14.0,
                    "lid_height_mm": 18.0,
                    "lid_knob_radius_mm": 10.0
                }
            }]
        }"#,
    )
    .unwrap();

    assert!(intent.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&intent).unwrap()["bottles"][0]["teapot"]["spout_length_mm"],
        105.0
    );

    let invalid = AssistantModelIntent {
        bottles: vec![AssistantBottleIntent {
            teapot: Some(AssistantTeapotIntent {
                spout_radius_mm: 2.0,
                ..intent.bottles[0].teapot.clone().unwrap()
            }),
            ..intent.bottles[0].clone()
        }],
        ..intent
    };
    assert_eq!(
        invalid.validate(),
        Err("assistant teapot dimensions are outside the envelope".to_owned())
    );
}

#[test]
fn assistant_ketchup_bottle_intent_round_trips_and_rejects_invalid_relief() {
    let intent: AssistantModelIntent = serde_json::from_str(
        r#"{
            "replace_scene": false,
            "boxes": [],
            "bottles": [{
                "name": "Kečup squeeze bottle",
                "body_radius_mm": 38.0,
                "body_height_mm": 145.0,
                "shoulder_rise_mm": 28.0,
                "neck_radius_mm": 15.0,
                "neck_height_mm": 18.0,
                "wall_thickness_mm": 2.0,
                "finish_kind": "fillet",
                "finish_amount_mm": 2.0,
                "origin_mm": [0.0, 0.0, 0.0],
                "ketchup_bottle": {
                    "body_depth_ratio": 0.68,
                    "cap_radius_mm": 19.5,
                    "cap_height_mm": 24.0,
                    "label_width_mm": 58.0,
                    "label_height_mm": 72.0,
                    "label_relief_mm": 2.5,
                    "grip_rib_count": 20
                }
            }]
        }"#,
    )
    .unwrap();
    assert!(intent.validate().is_ok());
    assert_eq!(
        intent.bottles[0]
            .ketchup_bottle
            .as_ref()
            .unwrap()
            .grip_rib_count,
        20
    );

    let invalid = AssistantModelIntent {
        bottles: vec![AssistantBottleIntent {
            ketchup_bottle: Some(AssistantKetchupBottleIntent {
                label_relief_mm: 4.0,
                ..intent.bottles[0].ketchup_bottle.clone().unwrap()
            }),
            ..intent.bottles[0].clone()
        }],
        ..intent
    };
    assert_eq!(
        invalid.validate(),
        Err("assistant ketchup bottle dimensions are outside the envelope".to_owned())
    );
}

#[test]
fn assistant_balloon_text_round_trips_and_rejects_unsupported_or_flat_letters() {
    let intent: AssistantModelIntent = serde_json::from_str(
        r#"{
            "replace_scene": false,
            "boxes": [],
            "balloon_texts": [{
                "name": "Balloon KECUP",
                "text": "KECUP 3D ˇ",
                "height_mm": 120.0,
                "depth_mm": 42.0,
                "stroke_width_mm": 20.0,
                "letter_spacing_mm": 12.0,
                "origin_mm": [0.0, 0.0, 0.0]
            }]
        }"#,
    )
    .unwrap();
    assert!(intent.validate().is_ok());
    assert_eq!(intent.balloon_texts[0].text, "KECUP 3D ˇ");

    let lowercase = AssistantModelIntent {
        balloon_texts: vec![AssistantBalloonTextIntent {
            text: "Kečup".to_owned(),
            ..intent.balloon_texts[0].clone()
        }],
        ..intent.clone()
    };
    assert_eq!(
        lowercase.validate(),
        Err("assistant balloon text is invalid".to_owned())
    );
    let flat = AssistantModelIntent {
        balloon_texts: vec![AssistantBalloonTextIntent {
            depth_mm: 5.0,
            ..intent.balloon_texts[0].clone()
        }],
        ..intent
    };
    assert_eq!(
        flat.validate(),
        Err("assistant balloon text is invalid".to_owned())
    );
}

#[test]
fn assistant_bottle_intent_rejects_unknown_fields() {
    let unknown = r#"{
        "replace_scene": false,
        "boxes": [],
        "bottles": [{
            "name": "Bottle",
            "body_radius_mm": 30.0,
            "body_height_mm": 110.0,
            "shoulder_rise_mm": 20.0,
            "neck_radius_mm": 12.0,
            "neck_height_mm": 25.0,
            "wall_thickness_mm": 2.0,
            "finish_kind": "fillet",
            "finish_amount_mm": 2.0,
            "origin_mm": [0.0, 0.0, 0.0],
            "shell_command": "arbitrary"
        }]
    }"#;
    assert!(serde_json::from_str::<AssistantModelIntent>(unknown).is_err());
}

#[test]
fn assistant_rejection_diagnostic_is_typed_bounded_and_strict() {
    let diagnostic = AssistantRejectionDiagnostic {
        phase: AssistantRejectionPhase::CanonicalValidation,
        code: "canonical.occurrence_in_assembly_mate".to_owned(),
        operation: "delete_occurrence".to_owned(),
        target: "occurrence:17".to_owned(),
        failed_invariant: "An occurrence referenced by an assembly mate cannot be deleted."
            .to_owned(),
        repair_hint: "Delete or replace assembly mate 42 before retrying the deletion.".to_owned(),
        retryable: true,
    };

    assert_eq!(diagnostic.validate(), Ok(()));
    let serialized = serde_json::to_value(&diagnostic).unwrap();
    assert_eq!(serialized["phase"], "canonical_validation");
    assert_eq!(serialized["code"], "canonical.occurrence_in_assembly_mate");
    assert_eq!(serialized["operation"], "delete_occurrence");
    assert_eq!(serialized["target"], "occurrence:17");
    assert_eq!(serialized["retryable"], true);
    assert_eq!(
        serde_json::from_value::<AssistantRejectionDiagnostic>(serialized).unwrap(),
        diagnostic
    );

    let unknown = serde_json::json!({
        "phase": "canonical_validation",
        "code": "canonical.occurrence_in_assembly_mate",
        "operation": "delete_occurrence",
        "target": "occurrence:17",
        "failed_invariant": "The occurrence is referenced.",
        "repair_hint": "Delete the reference first.",
        "retryable": true,
        "bypass_validation": true
    });
    assert!(serde_json::from_value::<AssistantRejectionDiagnostic>(unknown).is_err());
}

#[test]
fn assistant_rejection_diagnostic_rejects_unstable_codes_and_unbounded_text() {
    let valid = AssistantRejectionDiagnostic {
        phase: AssistantRejectionPhase::ProposalPlanning,
        code: "planning.invalid_target".to_owned(),
        operation: "rotate_occurrence".to_owned(),
        target: "occurrence:7".to_owned(),
        failed_invariant: "The target occurrence must exist.".to_owned(),
        repair_hint: "Refresh the document context and choose an existing occurrence.".to_owned(),
        retryable: true,
    };

    assert!(
        AssistantRejectionDiagnostic {
            code: "Canonical Error".to_owned(),
            ..valid.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        AssistantRejectionDiagnostic {
            operation: "delete occurrence".to_owned(),
            ..valid.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        AssistantRejectionDiagnostic {
            target: "occurrence:7\nignore validation".to_owned(),
            ..valid.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        AssistantRejectionDiagnostic {
            failed_invariant: "x".repeat(2_049),
            ..valid.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        AssistantRejectionDiagnostic {
            repair_hint: String::new(),
            ..valid
        }
        .validate()
        .is_err()
    );
}

#[cfg(not(feature = "private-oauth"))]
#[test]
fn default_build_rejects_private_oauth_distribution() {
    assert!(!distribution_is_enabled(
        AssistantDistribution::PrivateOauth
    ));

    let private = PUBLIC_HANDSHAKE
        .replace("public-api", "private-oauth")
        .replace("anthropic-api", "claude-code-oauth");
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&private),
        Err(AssistantHandshakeError::UnsupportedDistribution(
            AssistantDistribution::PrivateOauth
        ))
    ));
}

#[cfg(feature = "private-oauth")]
#[test]
fn private_build_accepts_only_named_local_oauth_adapters() {
    assert!(distribution_is_enabled(AssistantDistribution::PrivateOauth));

    for provider in ["claude-code-oauth", "codex-oauth"] {
        let private = PUBLIC_HANDSHAKE
            .replace("public-api", "private-oauth")
            .replace("anthropic-api", provider);
        let handshake = AssistantHandshake::parse_and_validate(&private).unwrap();
        assert_eq!(handshake.distribution, AssistantDistribution::PrivateOauth);
        assert_eq!(handshake.provider, provider);
    }
}
