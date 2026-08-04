use ketchup_core::adapters::{CliAdapter, RpcAdapter, RpcRequestV1, UiAction, UiAdapter};
use ketchup_core::document::{
    COMMAND_SCHEMA_V1, CanonicalCommand, CanonicalError, CommandBatch, Dimension, DocumentStore,
    NodeId, ProposalCommitError, ProposalValidity,
};
use ketchup_core::persistence;

const WIDTH: NodeId = NodeId(1);
const SHELF_DEPTH: NodeId = NodeId(2);
const INDEPENDENT: NodeId = NodeId(3);

fn dimension(token: &str, value: f64) -> Dimension {
    Dimension::new(token, value).expect("test dimension is valid")
}

fn seed_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateNode {
                id: WIDTH,
                name: "width".to_owned(),
                dimension: dimension("600", 600.0),
                dependencies: vec![],
            },
            CanonicalCommand::CreateNode {
                id: SHELF_DEPTH,
                name: "shelf_depth".to_owned(),
                dimension: dimension("width / 2", 300.0),
                dependencies: vec![WIDTH],
            },
            CanonicalCommand::CreateNode {
                id: INDEPENDENT,
                name: "leg_height".to_owned(),
                dimension: dimension("720", 720.0),
                dependencies: vec![],
            },
        ]))
        .expect("seed batch commits");
    document
}

#[test]
fn immutable_revisions_share_unchanged_nodes_and_recompute_only_dependents() {
    let mut document = seed_document();
    let before = document.current();
    let revision = document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetDimension {
            id: WIDTH,
            dimension: dimension("720", 720.0),
        }]))
        .expect("local parameter edit commits");
    let after = revision.snapshot();

    assert!(!before.shares_node_with(after, WIDTH));
    assert!(before.shares_node_with(after, INDEPENDENT));
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
        CanonicalCommand::RenameNode {
            id: WIDTH,
            name: "cabinet_width".to_owned(),
        },
        CanonicalCommand::SetDimension {
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
            CanonicalCommand::SetDimension {
                id: WIDTH,
                dimension: dimension("720", 720.0),
            },
            CanonicalCommand::RenameNode {
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
    let proposal =
        document.prepare_proposal(CommandBatch::new(vec![CanonicalCommand::SetDimension {
            id: WIDTH,
            dimension: dimension("720", 720.0),
        }]));

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameNode {
            id: INDEPENDENT,
            name: "tall_leg".to_owned(),
        }]))
        .expect("unrelated edit commits");
    assert!(matches!(
        document.validate_proposal(&proposal),
        ProposalValidity::Valid { .. }
    ));
    document
        .commit_proposal(&proposal)
        .expect("unchanged dependency digest permits the exact proposed batch");

    let stale =
        document.prepare_proposal(CommandBatch::new(vec![CanonicalCommand::SetDimension {
            id: WIDTH,
            dimension: dimension("840", 840.0),
        }]));
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetDimension {
            id: WIDTH,
            dimension: dimension("800", 800.0),
        }]))
        .expect("relevant edit commits");
    assert!(matches!(
        document.commit_proposal(&stale),
        Err(ProposalCommitError::Stale(ProposalValidity::Stale { .. }))
    ));
    assert_eq!(
        document
            .current()
            .node(WIDTH)
            .unwrap()
            .dimension()
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
        .node_ids()
        .map(|id| {
            let snapshot = document.current();
            let node = snapshot.node(id).unwrap();
            (
                id,
                node.name().to_owned(),
                node.dimension().source_token().to_owned(),
                node.dimension().millimetres().to_bits(),
            )
        })
        .collect();
    let mut bytes = persistence::save(&document.current());

    for _ in 0..100 {
        let loaded = persistence::load(&bytes).expect("current schema loads");
        assert_eq!(loaded.source_schema, 1);
        assert!(loaded.migration_losses.is_empty());
        let snapshot = loaded.document.current();
        assert_eq!(snapshot.canonical_digest(), expected_digest);
        for (id, name, token, bits) in &expected {
            let node = snapshot.node(*id).expect("stable node ID survives");
            assert_eq!(node.name(), name);
            assert_eq!(node.dimension().source_token(), token);
            assert_eq!(node.dimension().millimetres().to_bits(), *bits);
        }
        bytes = persistence::save(&snapshot);
    }
}

#[test]
fn ui_rpc_and_cli_actions_produce_identical_canonical_results() {
    let ui_batch = UiAdapter::canonicalize(UiAction::SetDimension {
        target: WIDTH,
        value_text: "720.125".to_owned(),
    })
    .expect("UI input canonicalizes");
    let rpc_batch = RpcAdapter::canonicalize(RpcRequestV1 {
        schema: COMMAND_SCHEMA_V1.to_owned(),
        method: "set_dimension".to_owned(),
        target: WIDTH.0,
        value: "720.125".to_owned(),
    })
    .expect("RPC input canonicalizes");
    let cli_batch = CliAdapter::canonicalize(&["set-dimension", "1", "720.125"])
        .expect("CLI input canonicalizes");

    assert_eq!(ui_batch, rpc_batch);
    assert_eq!(rpc_batch, cli_batch);
    assert_eq!(ui_batch.digest(), rpc_batch.digest());

    let mut ui_document = seed_document();
    let mut rpc_document = seed_document();
    let mut cli_document = seed_document();
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
                .map(|(id, name, token, value)| CanonicalCommand::CreateNode {
                    id: *id,
                    name: (*name).to_owned(),
                    dimension: dimension(token, *value),
                    dependencies: vec![],
                })
                .collect(),
        ))
        .unwrap();
    let expected_volume = values[0].3 * values[1].3 * values[2].3;
    let mut bytes = persistence::save(&document.current());

    for _ in 0..100 {
        let loaded = persistence::load(&bytes).unwrap();
        let snapshot = loaded.document.current();
        for (id, _, token, value) in values {
            let stored = snapshot.node(id).unwrap().dimension();
            assert_eq!(stored.source_token(), token);
            assert_eq!(stored.millimetres().to_bits(), value.to_bits());
        }
        let volume = snapshot.node(NodeId(10)).unwrap().dimension().millimetres()
            * snapshot.node(NodeId(11)).unwrap().dimension().millimetres()
            * snapshot.node(NodeId(12)).unwrap().dimension().millimetres();
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
    assert_eq!(loaded.source_schema, 0);
    assert_eq!(loaded.migration_losses.len(), 1);
    assert_eq!(loaded.migration_losses[0].node_id, NodeId(42));
    assert_eq!(loaded.migration_losses[0].field, "dimension.source_token");
    assert_eq!(
        loaded
            .document
            .current()
            .node(NodeId(42))
            .unwrap()
            .dimension()
            .millimetres()
            .to_bits(),
        480.25_f64.to_bits()
    );
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
