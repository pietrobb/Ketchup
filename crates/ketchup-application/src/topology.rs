use crate::diagnostics::{
    AssistantPlanningResult, assistant_canonical_rejection, assistant_planning_rejection,
};
use ketchup_model::document::{
    CanonicalError, ChamferEdgeSide, ChamferMode, DefinitionId, Dimension, DraftSpec,
    EdgeFinishKind, EdgeRef, FaceRef, FeatureId, FeatureKind, FilletRadiusStation, ShellDirection,
    Snapshot,
};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::tolerance::limits;
use ketchup_model::topology::{TopologicalElementKind, TopologicalElementRef};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralFinishKind {
    Shell,
    Fillet,
    Chamfer,
}

pub fn plan_topology_shell_kind(
    target: FeatureId,
    mut removed_faces: Vec<TopologicalElementRef>,
    thickness: Dimension,
    direction: ShellDirection,
) -> Option<FeatureKind> {
    if removed_faces.len() > limits::FEATURE_REFERENCES
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
    Some(FeatureKind::Shell {
        target,
        removed_faces: removed_faces.into_iter().map(FaceRef::from).collect(),
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
    if !(1..=limits::FEATURE_REFERENCES).contains(&references.len()) {
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
        GeneralFinishKind::Shell => FeatureKind::Shell {
            target,
            removed_faces: references.into_iter().map(FaceRef::from).collect(),
            thickness: amount,
            direction: ShellDirection::Inward,
        },
        GeneralFinishKind::Fillet | GeneralFinishKind::Chamfer => FeatureKind::EdgeFinish {
            target,
            edges: references.into_iter().map(EdgeRef::from).collect(),
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
    let FeatureKind::EdgeFinish {
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
    if !(1..=limits::FEATURE_REFERENCES).contains(&edge_sides.len())
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
        .map(|selection| EdgeRef::from(selection.edge.clone()))
        .collect();
    Some(FeatureKind::EdgeFinish {
        target,
        edges,
        kind: EdgeFinishKind::Chamfer,
        amount: distance,
        fillet_radius_stations: Vec::new(),
        chamfer_mode: mode,
        chamfer_edge_sides: edge_sides,
    })
}

/// A draft of 1 to 64 unique faces of `target`, none of them its neutral face.
/// The angle is checked by the document.
pub fn plan_topology_draft_kind(
    target: FeatureId,
    mut faces: Vec<TopologicalElementRef>,
    neutral: TopologicalElementRef,
    angle_degrees: f64,
) -> Option<FeatureKind> {
    if !(1..=limits::FEATURE_REFERENCES).contains(&faces.len())
        || faces.iter().chain([&neutral]).any(|reference| {
            reference.kind != TopologicalElementKind::Face
                || reference.producer_feature_id != target
                || !reference.has_valid_lineage()
        })
        || faces.contains(&neutral)
    {
        return None;
    }
    faces.sort_unstable();
    if faces.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    Some(FeatureKind::Draft(DraftSpec {
        target,
        faces: faces.into_iter().map(FaceRef::from).collect(),
        neutral: FaceRef::from(neutral),
        angle_degrees,
    }))
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

/// Resolves the target and the host-issued references of one topology edit.
pub(crate) struct TopologyPick<'a> {
    snapshot: &'a Snapshot,
    topology_results: &'a ExactResultRegistry,
    definition_id: DefinitionId,
    operation_name: &'a str,
}

impl<'a> TopologyPick<'a> {
    pub(crate) const fn new(
        snapshot: &'a Snapshot,
        topology_results: &'a ExactResultRegistry,
        definition_id: DefinitionId,
        operation_name: &'a str,
    ) -> Self {
        Self {
            snapshot,
            topology_results,
            definition_id,
            operation_name,
        }
    }

    /// The `finish` target: an unsuppressed feature of the definition that
    /// evaluates exactly and offers `kind` references.
    pub(crate) fn target(
        &self,
        target_feature_id: u64,
        finish: &str,
        kind: TopologicalElementKind,
    ) -> AssistantPlanningResult<FeatureId> {
        let target = FeatureId(target_feature_id);
        let place = format!("feature:{}", target.0);
        let source = self.snapshot.feature(target).ok_or_else(|| {
            assistant_canonical_rejection(
                CanonicalError::FeatureNotFound(target),
                self.operation_name,
                &place,
            )
        })?;
        if source.definition_id() != self.definition_id {
            return Err(assistant_planning_rejection(
                "planning.cad_feature_input_ownership_invalid",
                self.operation_name,
                &place,
                format!("The requested {finish} target belongs to a different definition."),
                "Target a supported exact body feature in the requested definition.",
            ));
        }
        if self.snapshot.feature_is_suppressed(target)
            || ExactBRepGraph::from_snapshot(self.snapshot, self.definition_id, target).is_err()
        {
            return Err(assistant_planning_rejection(
                "planning.cad_feature_input_unsupported",
                self.operation_name,
                &place,
                format!("The requested {finish} target is not supported by exact evaluation."),
                if kind == TopologicalElementKind::Face {
                    "Target an unsuppressed supported exact body feature with current host-issued face references."
                } else {
                    "Target an unsuppressed supported exact body feature with current host-issued edge references."
                },
            ));
        }
        Ok(target)
    }

    /// The one current host-issued `kind` reference of `target` for each id;
    /// `subject` names them in a refusal, such as "Shell face".
    pub(crate) fn references(
        &self,
        target: FeatureId,
        kind: TopologicalElementKind,
        reference_ids: &[String],
        subject: &str,
    ) -> AssistantPlanningResult<Vec<TopologicalElementRef>> {
        let available = assistant_topology_references(self.snapshot, self.topology_results, kind);
        reference_ids
            .iter()
            .map(|reference_id| self.unique(&available, target, kind, reference_id, subject))
            .collect()
    }

    /// `references` of a single id.
    pub(crate) fn reference(
        &self,
        target: FeatureId,
        kind: TopologicalElementKind,
        reference_id: &str,
        subject: &str,
    ) -> AssistantPlanningResult<TopologicalElementRef> {
        let available = assistant_topology_references(self.snapshot, self.topology_results, kind);
        self.unique(&available, target, kind, reference_id, subject)
    }

    fn unique(
        &self,
        available: &[&TopologicalElementRef],
        target: FeatureId,
        kind: TopologicalElementKind,
        reference_id: &str,
        subject: &str,
    ) -> AssistantPlanningResult<TopologicalElementRef> {
        let matches = available
            .iter()
            .copied()
            .filter(|reference| {
                reference.definition_id == self.definition_id
                    && reference.producer_feature_id == target
                    && reference.lineage_digest == reference_id
            })
            .collect::<Vec<_>>();
        let [reference] = matches.as_slice() else {
            return Err(assistant_planning_rejection(
                "planning.cad_topology_reference_unavailable",
                self.operation_name,
                &format!("feature:{}", target.0),
                format!(
                    "A requested {subject} reference is not a unique current host-issued reference for the target."
                ),
                if kind == TopologicalElementKind::Face {
                    "Refresh the document context and use only listed topology_face_references for this target."
                } else {
                    "Refresh the document context and use only listed topology_edge_references for this target."
                },
            ));
        };
        Ok((*reference).clone())
    }
}
