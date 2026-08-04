use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    GroupId, NodeId, OccurrenceId, TagId, Transform,
};
use ketchup_core::state_view::{
    AGENT_STATE_VIEW_V1, COMPLETE_STATE_VIEW_V1, encode_semantic_state,
};
use std::path::PathBuf;

fn fixture_document(reverse_nodes: bool) -> DocumentStore {
    let nodes = if reverse_nodes {
        vec![
            CanonicalCommand::CreateNode {
                id: NodeId(2),
                name: "Dependent".to_owned(),
                dimension: Dimension::new("2.500", 2.5).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateNode {
                id: NodeId(1),
                name: "Width \"quoted\"".to_owned(),
                dimension: Dimension::new("100.00", 100.0).unwrap(),
                dependencies: vec![],
            },
        ]
    } else {
        vec![
            CanonicalCommand::CreateNode {
                id: NodeId(1),
                name: "Width \"quoted\"".to_owned(),
                dimension: Dimension::new("100.00", 100.0).unwrap(),
                dependencies: vec![],
            },
            CanonicalCommand::CreateNode {
                id: NodeId(2),
                name: "Dependent".to_owned(),
                dimension: Dimension::new("2.500", 2.5).unwrap(),
                dependencies: vec![],
            },
        ]
    };
    let mut commands = nodes;
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(10),
            name: "Cabinet\\nA".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(11),
            definition_id: DefinitionId(10),
            name: "Rectangle".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [600.0, 0.0], [600.0, 580.0], [0.0, 580.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(12),
            definition_id: DefinitionId(10),
            name: "Extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(11),
                height: Dimension::new("720.000", 720.0).unwrap(),
            },
        },
        CanonicalCommand::CreateGroup {
            id: GroupId(20),
            name: "Kitchen run".to_owned(),
            transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
            parent: None,
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(30),
            definition_id: DefinitionId(10),
            name: "Cabinet #1".to_owned(),
            transform: Transform::identity(),
            parent: Some(GroupId(20)),
            tag: Some(TagId(7)),
            visible: true,
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(31),
            definition_id: DefinitionId(10),
            name: "Cabinet #2".to_owned(),
            transform: Transform::from_translation(700.0, 0.0, 0.0).unwrap(),
            parent: None,
            tag: None,
            visible: false,
        },
    ]);
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    store
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("state-view")
        .join(name)
}

fn assert_golden(name: &str, actual: &str) {
    let path = fixture_path(name);
    if std::env::var_os("UPDATE_STATE_VIEW_FIXTURES").is_some() {
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap();
    assert_eq!(actual, expected, "StateView drift in {}", path.display());
}

#[test]
fn complete_and_agent_v1_match_independently_versioned_golden_fixtures() {
    assert_ne!(COMPLETE_STATE_VIEW_V1, AGENT_STATE_VIEW_V1);
    let state = encode_semantic_state(&fixture_document(false).current());
    let complete = state.complete_v1();
    let agent = state.agent_v1();

    assert_golden("complete-v1.txt", &complete);
    assert_golden("agent-v1.txt", &agent);
    assert!(complete.contains("height.f64_bits=4086800000000000"));
    assert!(complete.contains("occurrence.30.transform.f64_bits="));
    assert!(!agent.contains("transform.f64_bits"));
    assert!(agent.contains("validation_health=not_evaluated"));
}

#[test]
fn one_encoder_is_deterministic_and_complete_output_detects_semantic_drift() {
    let first = fixture_document(false);
    let reordered = fixture_document(true);
    assert_eq!(
        encode_semantic_state(&first.current()).complete_v1(),
        encode_semantic_state(&reordered.current()).complete_v1()
    );

    let before = encode_semantic_state(&first.current()).complete_v1();
    let mut changed = first;
    changed
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(31),
                visible: true,
            },
        ]))
        .unwrap();
    let after = encode_semantic_state(&changed.current()).complete_v1();
    assert_ne!(before, after);
    assert!(after.contains("occurrence.31.visible=true"));
}
