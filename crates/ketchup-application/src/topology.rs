use ketchup_core::document::{Dimension, EdgeFinishKind, FeatureId, FeatureKind, Snapshot};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::topology::{TopologicalElementKind, TopologicalElementRef};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFinishKind {
    Shell,
    Fillet,
    Chamfer,
}

pub const MAX_TOPOLOGICAL_FINISH_REFERENCES: usize = 64;

pub fn plan_topology_finish_kind(
    kind: GeneralFinishKind,
    target: FeatureId,
    mut references: Vec<TopologicalElementRef>,
    amount: Dimension,
) -> Option<FeatureKind> {
    if !(1..=MAX_TOPOLOGICAL_FINISH_REFERENCES).contains(&references.len()) {
        return None;
    }
    let expected_kind = match kind {
        GeneralFinishKind::Shell => TopologicalElementKind::Face,
        GeneralFinishKind::Fillet | GeneralFinishKind::Chamfer => TopologicalElementKind::Edge,
    };
    if references.iter().any(|reference| {
        reference.kind != expected_kind
            || reference.producer_feature_id != target
            || !reference.has_valid_lineage()
    }) {
        return None;
    }
    references.sort_unstable();
    if references.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    Some(match kind {
        GeneralFinishKind::Shell => FeatureKind::TopologyShell {
            target,
            removed_faces: references,
            thickness: amount,
        },
        GeneralFinishKind::Fillet | GeneralFinishKind::Chamfer => FeatureKind::TopologyEdgeFinish {
            target,
            edges: references,
            kind: if kind == GeneralFinishKind::Fillet {
                EdgeFinishKind::Fillet
            } else {
                EdgeFinishKind::Chamfer
            },
            amount,
        },
    })
}

pub fn assistant_topology_references<'a>(
    snapshot: &'a Snapshot,
    topology_results: &'a ExactResultRegistry,
    kind: TopologicalElementKind,
) -> Vec<&'a TopologicalElementRef> {
    let mut references = topology_results
        .body_values(snapshot)
        .unwrap_or_default()
        .into_values()
        .flat_map(|package| package.topological_references())
        .filter(|reference| {
            reference.kind == kind
                && reference.document_id == snapshot.document_id()
                && reference.source_feature_id == reference.producer_feature_id
                && reference.has_valid_lineage()
        })
        .collect::<Vec<_>>();
    references.sort_unstable();
    references
}
