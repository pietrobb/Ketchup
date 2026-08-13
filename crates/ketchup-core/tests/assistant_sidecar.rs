use ketchup_core::assistant_sidecar::{
    ASSISTANT_PROTOCOL_VERSION, AssistantDistribution, AssistantHandshake, AssistantHandshakeError,
    AssistantLinearArrayIntent, AssistantModelIntent, distribution_is_enabled,
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
        linear_arrays: vec![AssistantLinearArrayIntent {
            occurrence_ids: (1..=100).collect(),
            instances: 6,
            step_mm: [0.0, 0.0, 280.0],
        }],
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
