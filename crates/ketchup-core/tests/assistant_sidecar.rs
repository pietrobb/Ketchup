use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantBalloonTextIntent, AssistantBeamNotchIntent,
    AssistantBottleFinishKind, AssistantBottleIntent, AssistantDistribution, AssistantHandshake,
    AssistantHandshakeError, AssistantKetchupBottleIntent, AssistantLinearArrayIntent,
    AssistantModelIntent, AssistantOrientedBeamIntent, AssistantParameterEditIntent,
    AssistantProfileTranslationIntent, AssistantTeapotIntent, distribution_is_enabled,
};

const PUBLIC_HANDSHAKE: &str = r#"{
    "protocol_version": 2,
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

    let version = PUBLIC_HANDSHAKE.replace("\"protocol_version\": 2", "\"protocol_version\": 3");
    assert!(matches!(
        AssistantHandshake::parse_and_validate(&version),
        Err(AssistantHandshakeError::UnsupportedProtocolVersion(3))
    ));
}

#[test]
fn assistant_array_budget_matches_the_canonical_proposal_command_limit() {
    let valid = AssistantModelIntent {
        replace_scene: false,
        boxes: Vec::new(),
        translations: Vec::new(),
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
