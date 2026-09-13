use ketchup_core::document::{
    ChamferEdgeSide, ChamferMode, Dimension, EdgeFinishKind, FeatureId, FeatureKind,
    FilletRadiusStation, ShellDirection, Snapshot,
};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::topology::{TopologicalElementKind, TopologicalElementRef};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFinishKind {
    Shell,
    Fillet,
    Chamfer,
}

pub const MAX_TOPOLOGICAL_FINISH_REFERENCES: usize = 64;

pub fn plan_topology_shell_kind(
    target: FeatureId,
    mut removed_faces: Vec<TopologicalElementRef>,
    thickness: Dimension,
    direction: ShellDirection,
) -> Option<FeatureKind> {
    if removed_faces.len() > MAX_TOPOLOGICAL_FINISH_REFERENCES
        || removed_faces.iter().any(|reference| {
            reference.kind != TopologicalElementKind::Face
                || reference.producer_feature_id != target
                || !reference.has_valid_lineage()
        })
    {
        return None;
    }
    removed_faces.sort_unstable();
    if removed_faces.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    Some(FeatureKind::TopologyShell {
        target,
        removed_faces,
        thickness,
        direction,
    })
}

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
            direction: ShellDirection::Inward,
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
            fillet_radius_stations: Vec::new(),
            chamfer_mode: ChamferMode::Symmetric,
            chamfer_edge_sides: Vec::new(),
        },
    })
}

pub fn plan_topology_variable_fillet_kind(
    target: FeatureId,
    references: Vec<TopologicalElementRef>,
    start_radius: Dimension,
    radius_stations: Vec<FilletRadiusStation>,
) -> Option<FeatureKind> {
    let mut feature =
        plan_topology_finish_kind(GeneralFinishKind::Fillet, target, references, start_radius)?;
    let FeatureKind::TopologyEdgeFinish {
        fillet_radius_stations,
        ..
    } = &mut feature
    else {
        unreachable!("Fillet planning produces an edge finish")
    };
    *fillet_radius_stations = radius_stations;
    Some(feature)
}

pub fn plan_topology_advanced_chamfer_kind(
    target: FeatureId,
    mut edge_sides: Vec<ChamferEdgeSide>,
    distance: Dimension,
    mode: ChamferMode,
) -> Option<FeatureKind> {
    if !(1..=MAX_TOPOLOGICAL_FINISH_REFERENCES).contains(&edge_sides.len())
        || matches!(mode, ChamferMode::Symmetric)
        || edge_sides.iter().any(|selection| {
            selection.edge.kind != TopologicalElementKind::Edge
                || selection.side_face.kind != TopologicalElementKind::Face
                || selection.edge.producer_feature_id != target
                || selection.side_face.producer_feature_id != target
                || selection.edge.definition_id != selection.side_face.definition_id
                || !selection.edge.has_valid_lineage()
                || !selection.side_face.has_valid_lineage()
        })
    {
        return None;
    }
    match &mode {
        ChamferMode::TwoDistance { second_distance } => {
            let value = second_distance.millimetres();
            if !value.is_finite() || !(0.01..=100_000.0).contains(&value) {
                return None;
            }
        }
        ChamferMode::DistanceAngle { angle_degrees } => {
            if !angle_degrees.is_finite() || *angle_degrees <= 0.1 || *angle_degrees >= 89.9 {
                return None;
            }
        }
        ChamferMode::Symmetric => return None,
    }
    edge_sides.sort_unstable_by(|left, right| left.edge.cmp(&right.edge));
    if edge_sides
        .windows(2)
        .any(|pair| pair[0].edge == pair[1].edge)
    {
        return None;
    }
    let edges = edge_sides
        .iter()
        .map(|selection| selection.edge.clone())
        .collect();
    Some(FeatureKind::TopologyEdgeFinish {
        target,
        edges,
        kind: EdgeFinishKind::Chamfer,
        amount: distance,
        fillet_radius_stations: Vec::new(),
        chamfer_mode: mode,
        chamfer_edge_sides: edge_sides,
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
