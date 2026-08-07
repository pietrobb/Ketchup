use ketchup_core::adapters::{CliAdapter, RpcAdapter, RpcRequestV1, UiAction, UiAdapter};
use ketchup_core::document::{
    COMMAND_SCHEMA_V1, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, NodeId, ProposalCommitError, ProposalValidity,
};
use ketchup_core::persistence;

const WIDTH: NodeId = NodeId(1);
const SHELF_DEPTH: NodeId = NodeId(2);
const INDEPENDENT: NodeId = NodeId(3);
const PRODUCT_DEFINITION: DefinitionId = DefinitionId(10);
const PRODUCT_PROFILE: FeatureId = FeatureId(11);
const PRODUCT_EXTRUSION: FeatureId = FeatureId(12);

fn dimension(token: &str, value: f64) -> Dimension {
    Dimension::new(token, value).expect("test dimension is valid")
}

fn seed_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: WIDTH,
                name: "width".to_owned(),
                dimension: dimension("600", 600.0),
                dependencies: vec![],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: SHELF_DEPTH,
                name: "shelf_depth".to_owned(),
                dimension: dimension("width / 2", 300.0),
                dependencies: vec![WIDTH],
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: INDEPENDENT,
                name: "leg_height".to_owned(),
                dimension: dimension("720", 720.0),
                dependencies: vec![],
            },
        ]))
        .expect("seed batch commits");
    document
}

fn seed_product_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: PRODUCT_DEFINITION,
                name: "Box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PRODUCT_PROFILE,
                definition_id: PRODUCT_DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: PRODUCT_EXTRUSION,
                definition_id: PRODUCT_DEFINITION,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PRODUCT_PROFILE,
                    height: dimension("600", 600.0),
                },
            },
        ]))
        .expect("product seed batch commits");
    document
}

#[test]
fn immutable_revisions_share_unchanged_nodes_and_recompute_only_dependents() {
    let mut document = seed_document();
    let before = document.current();
    let revision = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: WIDTH,
                dimension: dimension("720", 720.0),
            },
        ]))
        .expect("local parameter edit commits");
    let after = revision.snapshot();

    assert!(!before.shares_evaluator_node_with(after, WIDTH));
    assert!(before.shares_evaluator_node_with(after, INDEPENDENT));
    assert_eq!(revision.recomputed_nodes().len(), 2);
    assert!(revision.recomputed_nodes().contains(&WIDTH));
    assert!(revision.recomputed_nodes().contains(&SHELF_DEPTH));
    assert!(!revision.recomputed_nodes().contains(&INDEPENDENT));
}

#[test]
fn command_batch_is_atomic_and_is_one_undo_redo_step() {
    let mut document = seed_document();
    let prior_digest = document.current().canonical_digest();
    let prior_revision_count = document.revision_count();

    let invalid = CommandBatch::new(vec![
        CanonicalCommand::RenameEvaluatorNode {
            id: WIDTH,
            name: "cabinet_width".to_owned(),
        },
        CanonicalCommand::SetEvaluatorDimension {
            id: NodeId(999),
            dimension: dimension("10", 10.0),
        },
    ]);
    assert!(matches!(
        document.apply_batch(&invalid),
        Err(CanonicalError::NodeNotFound(NodeId(999)))
    ));
    assert_eq!(document.current().canonical_digest(), prior_digest);
    assert_eq!(document.revision_count(), prior_revision_count);

    let committed = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: WIDTH,
                dimension: dimension("720", 720.0),
            },
            CanonicalCommand::RenameEvaluatorNode {
                id: WIDTH,
                name: "cabinet_width".to_owned(),
            },
        ]))
        .expect("valid multi-command batch commits");
    let committed_digest = committed.snapshot().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 2);
    assert_eq!(document.undo().unwrap().canonical_digest(), prior_digest);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn proposal_revalidation_accepts_unrelated_edits_and_rejects_changed_dependencies() {
    let mut document = seed_document();
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: WIDTH,
                dimension: dimension("720", 720.0),
            },
        ]))
        .unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameEvaluatorNode {
                id: INDEPENDENT,
                name: "tall_leg".to_owned(),
            },
        ]))
        .expect("unrelated edit commits");
    assert!(matches!(
        document.validate_proposal(&proposal),
        ProposalValidity::Valid { .. }
    ));
    document
        .commit_proposal(&proposal)
        .expect("unchanged dependency digest permits the exact proposed batch");

    let stale = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: WIDTH,
                dimension: dimension("840", 840.0),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: WIDTH,
                dimension: dimension("800", 800.0),
            },
        ]))
        .expect("relevant edit commits");
    assert!(matches!(
        document.commit_proposal(&stale),
        Err(ProposalCommitError::Stale(ProposalValidity::Stale { .. }))
    ));
    assert_eq!(
        document
            .current()
            .evaluator_node(WIDTH)
            .unwrap()
            .dimension()
            .unwrap()
            .millimetres(),
        800.0
    );
}

#[test]
fn canonical_ids_and_parameters_survive_one_hundred_save_load_cycles() {
    let document = seed_document();
    let expected_digest = document.current().canonical_digest();
    let expected: Vec<_> = document
        .current()
        .evaluator_node_ids()
        .map(|id| {
            let snapshot = document.current();
            let node = snapshot.evaluator_node(id).unwrap();
            (
                id,
                node.name().to_owned(),
                node.dimension().unwrap().source_token().to_owned(),
                node.dimension().unwrap().millimetres().to_bits(),
            )
        })
        .collect();
    let mut bytes = persistence::save(&document.current());

    for _ in 0..100 {
        let loaded = persistence::load(&bytes).expect("current schema loads");
        assert_eq!(loaded.source_schema(), 10);
        assert!(loaded.migration_losses().is_empty());
        let snapshot = loaded.snapshot();
        assert_eq!(snapshot.canonical_digest(), expected_digest);
        for (id, name, token, bits) in &expected {
            let node = snapshot
                .evaluator_node(*id)
                .expect("stable node ID survives");
            assert_eq!(node.name(), name);
            assert_eq!(node.dimension().unwrap().source_token(), token);
            assert_eq!(node.dimension().unwrap().millimetres().to_bits(), *bits);
        }
        bytes = persistence::save(&snapshot);
    }
}

#[test]
fn ui_rpc_and_cli_actions_produce_identical_canonical_results() {
    let ui_batch = UiAdapter::canonicalize(UiAction::SetFeatureDimension {
        target: PRODUCT_EXTRUSION,
        value_text: "720.125".to_owned(),
    })
    .expect("UI input canonicalizes");
    let rpc_batch = RpcAdapter::canonicalize(RpcRequestV1 {
        schema: COMMAND_SCHEMA_V1.to_owned(),
        method: "set_feature_dimension".to_owned(),
        target: PRODUCT_EXTRUSION.0,
        value: "720.125".to_owned(),
    })
    .expect("RPC input canonicalizes");
    let cli_batch = CliAdapter::canonicalize(&["set-feature-dimension", "12", "720.125"])
        .expect("CLI input canonicalizes");

    assert_eq!(ui_batch, rpc_batch);
    assert_eq!(rpc_batch, cli_batch);
    assert_eq!(ui_batch.digest(), rpc_batch.digest());

    let seed = seed_product_document();
    let bytes = persistence::save(&seed.current());
    let mut ui_document = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let mut rpc_document = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let mut cli_document = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    ui_document.apply_batch(&ui_batch).unwrap();
    rpc_document.apply_batch(&rpc_batch).unwrap();
    cli_document.apply_batch(&cli_batch).unwrap();
    let ui_digest = ui_document.current().canonical_digest();
    assert_eq!(ui_digest, rpc_document.current().canonical_digest());
    assert_eq!(ui_digest, cli_document.current().canonical_digest());
}

#[test]
fn precision_values_do_not_drift_and_derived_geometry_stays_within_tolerance() {
    let values = [
        (NodeId(10), "width", "100.125", 100.125),
        (NodeId(11), "depth", "60.0625", 60.0625),
        (NodeId(12), "height", "20.03125", 20.03125),
        (NodeId(13), "minimum", "0.01", 0.01),
        (NodeId(14), "maximum", "100000", 100_000.0),
    ];
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(
            values
                .iter()
                .map(
                    |(id, name, token, value)| CanonicalCommand::CreateEvaluatorNode {
                        id: *id,
                        name: (*name).to_owned(),
                        dimension: dimension(token, *value),
                        dependencies: vec![],
                    },
                )
                .collect(),
        ))
        .unwrap();
    let expected_volume = values[0].3 * values[1].3 * values[2].3;
    let mut bytes = persistence::save(&document.current());

    for _ in 0..100 {
        let loaded = persistence::load(&bytes).unwrap();
        let snapshot = loaded.snapshot();
        for (id, _, token, value) in values {
            let stored = snapshot.evaluator_node(id).unwrap().dimension().unwrap();
            assert_eq!(stored.source_token(), token);
            assert_eq!(stored.millimetres().to_bits(), value.to_bits());
        }
        let volume = snapshot
            .evaluator_node(NodeId(10))
            .unwrap()
            .dimension()
            .unwrap()
            .millimetres()
            * snapshot
                .evaluator_node(NodeId(11))
                .unwrap()
                .dimension()
                .unwrap()
                .millimetres()
            * snapshot
                .evaluator_node(NodeId(12))
                .unwrap()
                .dimension()
                .unwrap()
                .millimetres();
        assert!((volume - expected_volume).abs() <= 0.000_001);
        bytes = persistence::save(&snapshot);
    }
}

#[test]
fn legacy_schema_preserves_binary_meaning_and_reports_unrecoverable_source_token() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"KETCHUPDOC");
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&7_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&42_u64.to_le_bytes());
    push_string(&mut bytes, "legacy_width");
    bytes.extend_from_slice(&480.25_f64.to_bits().to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());

    let loaded = persistence::load(&bytes).expect("legacy fixture migrates");
    assert_eq!(loaded.source_schema(), 0);
    assert_eq!(loaded.migration_losses().len(), 1);
    assert_eq!(loaded.migration_losses()[0].node_id, NodeId(42));
    assert_eq!(loaded.migration_losses()[0].field, "dimension.source_token");
    assert_eq!(
        loaded
            .snapshot()
            .evaluator_node(NodeId(42))
            .unwrap()
            .dimension()
            .unwrap()
            .millimetres()
            .to_bits(),
        480.25_f64.to_bits()
    );
}

#[test]
fn product_proposals_use_typed_dependencies_without_snapshot_fallback() {
    let mut document = seed_product_document();
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: PRODUCT_EXTRUSION,
                dimension: dimension("720", 720.0),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(99),
                name: "Unrelated".to_owned(),
            },
        ]))
        .unwrap();
    assert!(matches!(
        document.validate_proposal(&proposal),
        ProposalValidity::Valid { .. }
    ));
    document.commit_proposal(&proposal).unwrap();

    let stale = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: PRODUCT_EXTRUSION,
                dimension: dimension("840", 840.0),
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PRODUCT_PROFILE,
                points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0]],
            },
        ]))
        .unwrap();
    assert!(matches!(
        document.validate_proposal(&stale),
        ProposalValidity::Stale { .. }
    ));
}

#[test]
fn schema_zero_and_one_are_one_way_imports_into_schema_three_authority() {
    for schema in [0_u16, 1_u16] {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"KETCHUPDOC");
        bytes.extend_from_slice(&schema.to_le_bytes());
        bytes.extend_from_slice(&7_u64.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&42_u64.to_le_bytes());
        push_string(&mut bytes, "imported_width");
        if schema == 1 {
            push_string(&mut bytes, "480.25");
        }
        bytes.extend_from_slice(&480.25_f64.to_bits().to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        let loaded = persistence::load(&bytes).expect("old schema imports");
        assert_eq!(loaded.source_schema(), schema);
        if schema == 0 {
            assert!(!loaded.is_editable());
            assert_eq!(
                loaded
                    .snapshot()
                    .evaluator_node(NodeId(42))
                    .unwrap()
                    .dimension()
                    .unwrap()
                    .millimetres(),
                480.25
            );
            continue;
        }
        let mut document = loaded.into_editable().ok().unwrap();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(42),
                    dimension: dimension("500", 500.0),
                },
            ]))
            .expect("the imported ProductModel is the writable authority");
        let saved = persistence::save(&document.current());
        assert_eq!(u16::from_le_bytes([saved[10], saved[11]]), 10);
        let reopened = persistence::load(&saved).expect("current schema reopens");
        assert_eq!(reopened.source_schema(), 10);
        assert_eq!(
            reopened
                .snapshot()
                .evaluator_node(NodeId(42))
                .unwrap()
                .dimension()
                .unwrap()
                .millimetres(),
            500.0
        );
    }
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
