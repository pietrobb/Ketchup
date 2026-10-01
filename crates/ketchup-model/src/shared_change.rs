#![forbid(unsafe_code)]

use crate::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyRecomputeStatus,
    AssemblyReferenceHealth, AssemblySolveStatus, AssemblySolverPolicy,
    recompute_rigid_assembly_mates_from_snapshot,
};
use crate::document::DocumentStore;
use crate::document::{
    AuthoritativeDependency, BodyId, CanonicalCommand, CanonicalError, CloneDefinitionPlan,
    CollectionId, CommandBatch, DefinitionId, DocumentId, FeatureId, GroupId, InstancePath,
    OccurrenceId, Proposal, ProposalAssumption, ProposalBudget, ProposalCommitError,
    ProposalConfirmation, ProposalContext, ProposalGoal, ProposalPrincipal, ProposalRisk, Snapshot,
    Transform,
};
use crate::drawing::{
    DrawingSheet, DrawingSheetId, DrawingSource, OrthographicDrawing, OrthographicViewKind,
    project_orthographic_drawing, validate_source,
};
use crate::exact_brep_graph::ExactBRepGraph;
use crate::exact_product::{
    BodySubshapeRef, EXACT_BREP_GRAPH_EVALUATOR_V1, ExactBodyPackage, ExactProductError,
    ExactReferenceResolution, ExactResultRegistry, body_exact_graph,
    canonical_reference_lineage_digest, exact_model_stl_export, producer_exact_graph,
    terminal_body_exact_graphs,
};
use crate::feature_history::{
    BodyHistoryMutationRequest, BodyParameterEditRequest, prepare_body_history_mutation,
    prepare_body_parameter_edit, prepare_dependency_staging_body_parameter_edit,
};
use crate::tolerance::ROUNDING;
use std::fmt;
use std::sync::Arc;

mod dependency;
mod replacement;
pub use dependency::{CandidateDrift, DependencyBlocker, ExportProblem, MateProblem, SheetProblem};
pub use replacement::{
    ComponentReplacementBodyCorrespondence, ComponentReplacementCommitError,
    ComponentReplacementCommitReceipt, ComponentReplacementFeatureCorrespondence,
    ComponentReplacementImpactError, ComponentReplacementImpactProjection,
    ComponentReplacementImpactRequest, ComponentReplacementMateReferenceImpact,
    ComponentReplacementSubshapeCorrespondence, FeatureMismatch, ReplacementMismatch,
    ReviewMismatch, SubshapeMismatch, commit_component_replacement,
    project_component_replacement_impact, project_component_replacement_impact_for_principal,
};

pub const OCCURRENCE_EDIT_IMPACT_SCHEMA_V1: &str = "ketchup.occurrence-edit-impact.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OccurrenceEdit {
    Delete,
    Reparent {
        parent: Option<GroupId>,
        preserve_world_transform: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceEditRequest {
    pub source_revision: u64,
    pub source_digest: String,
    pub target_occurrence_id: OccurrenceId,
    pub edit: OccurrenceEdit,
}

impl OccurrenceEditRequest {
    #[must_use]
    pub fn delete(snapshot: &Snapshot, target_occurrence_id: OccurrenceId) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            target_occurrence_id,
            edit: OccurrenceEdit::Delete,
        }
    }

    #[must_use]
    pub fn reparent(
        snapshot: &Snapshot,
        target_occurrence_id: OccurrenceId,
        parent: Option<GroupId>,
    ) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            target_occurrence_id,
            edit: OccurrenceEdit::Reparent {
                parent,
                preserve_world_transform: true,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrenceCollectionDependencyImpact {
    pub collection_id: CollectionId,
    pub occurrence_ids_before: Vec<OccurrenceId>,
    pub occurrence_ids_after: Vec<OccurrenceId>,
}

fn preserved_collection_dependencies(
    snapshot: &Snapshot,
    occurrence_id: OccurrenceId,
) -> Vec<OccurrenceCollectionDependencyImpact> {
    let mut dependencies = snapshot
        .collections()
        .filter_map(|collection| {
            let occurrence_ids = collection.occurrence_ids().collect::<Vec<_>>();
            occurrence_ids
                .contains(&occurrence_id)
                .then(|| OccurrenceCollectionDependencyImpact {
                    collection_id: collection.id(),
                    occurrence_ids_before: occurrence_ids.clone(),
                    occurrence_ids_after: occurrence_ids,
                })
        })
        .collect::<Vec<_>>();
    dependencies.sort_by_key(|dependency| dependency.collection_id);
    dependencies
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OccurrenceDrawingDependencyAction {
    UpdateRigidAssembly { occurrence_ids: Vec<OccurrenceId> },
    DeleteSheet,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrenceDrawingDependencyImpact {
    pub sheet_id: DrawingSheetId,
    pub occurrence_ids_before: Vec<OccurrenceId>,
    pub action: OccurrenceDrawingDependencyAction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceEditImpactProjection {
    pub schema: &'static str,
    pub source_revision: u64,
    pub source_digest: String,
    pub candidate_digest: String,
    pub target_occurrence_id: OccurrenceId,
    pub target_instance_path: InstancePath,
    pub edit: OccurrenceEdit,
    pub parent_before: Option<GroupId>,
    pub parent_after: Option<GroupId>,
    pub local_transform_before: Transform,
    pub local_transform_after: Option<Transform>,
    pub world_transform_before: Transform,
    pub world_transform_after: Option<Transform>,
    pub incident_mate_ids: Vec<AssemblyMateId>,
    pub collection_dependencies: Vec<OccurrenceCollectionDependencyImpact>,
    pub drawing_dependencies: Vec<OccurrenceDrawingDependencyImpact>,
    pub proposal: Proposal,
}

impl OccurrenceEditImpactProjection {
    #[must_use]
    pub fn is_review_only(&self) -> bool {
        matches!(
            self.proposal.confirmation(),
            ProposalConfirmation::ReviewRequired
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OccurrenceEditImpactError {
    Stale,
    OccurrenceNotFound(OccurrenceId),
    ParentNotFound(GroupId),
    UnchangedParent(OccurrenceId),
    NonInvertibleParent(GroupId),
    InvalidCandidate(String),
}

impl fmt::Display for OccurrenceEditImpactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("occurrence edit request is stale"),
            Self::OccurrenceNotFound(id) => write!(formatter, "occurrence {} was not found", id.0),
            Self::ParentNotFound(id) => write!(formatter, "parent group {} was not found", id.0),
            Self::UnchangedParent(id) => {
                write!(
                    formatter,
                    "occurrence {} already has the requested parent",
                    id.0
                )
            }
            Self::NonInvertibleParent(id) => write!(
                formatter,
                "parent group {} does not have an invertible world transform",
                id.0
            ),
            Self::InvalidCandidate(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for OccurrenceEditImpactError {}

pub fn project_occurrence_edit_impact(
    document: &DocumentStore,
    request: OccurrenceEditRequest,
    principal: ProposalPrincipal,
) -> Result<OccurrenceEditImpactProjection, OccurrenceEditImpactError> {
    let source = document.current();
    if request.source_revision != source.revision_id()
        || request.source_digest != source.canonical_digest()
    {
        return Err(OccurrenceEditImpactError::Stale);
    }
    let target = source.occurrence(request.target_occurrence_id).ok_or(
        OccurrenceEditImpactError::OccurrenceNotFound(request.target_occurrence_id),
    )?;
    let target_instance_path = InstancePath::root(request.target_occurrence_id);
    let parent_before = target.parent();
    let local_transform_before = target.transform();
    let world_transform_before = source
        .world_transform_for_occurrence(request.target_occurrence_id)
        .ok_or_else(|| {
            OccurrenceEditImpactError::InvalidCandidate(
                "target occurrence has no valid world transform".to_owned(),
            )
        })?;

    let mut incident_mate_ids = source
        .assembly_mates()
        .filter(|mate| {
            mate.endpoint_a().occurrence_id() == request.target_occurrence_id
                || mate.endpoint_b().occurrence_id() == request.target_occurrence_id
        })
        .map(AssemblyMate::id)
        .collect::<Vec<_>>();
    incident_mate_ids.sort_unstable();

    if let OccurrenceEdit::Reparent {
        parent,
        preserve_world_transform,
    } = request.edit
    {
        if !preserve_world_transform {
            return Err(OccurrenceEditImpactError::InvalidCandidate(
                "occurrence reparent must preserve its world transform".to_owned(),
            ));
        }
        if parent == parent_before {
            return Err(OccurrenceEditImpactError::UnchangedParent(
                request.target_occurrence_id,
            ));
        }
        let parent_world_transform = match parent {
            Some(id) => {
                if source.group(id).is_none() {
                    return Err(OccurrenceEditImpactError::ParentNotFound(id));
                }
                source
                    .world_transform_for_group(id)
                    .ok_or(OccurrenceEditImpactError::ParentNotFound(id))?
            }
            None => Transform::identity(),
        };
        let inverse_parent = parent_world_transform.inverse().ok_or_else(|| {
            parent.map_or_else(
                || {
                    OccurrenceEditImpactError::InvalidCandidate(
                        "root world transform is not invertible".to_owned(),
                    )
                },
                OccurrenceEditImpactError::NonInvertibleParent,
            )
        })?;
        let local_transform_after = inverse_parent.compose(world_transform_before);
        let commands = vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: request.target_occurrence_id,
                transform: local_transform_after,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: request.target_occurrence_id,
                parent,
            },
        ];
        let mut assumptions = vec![ProposalAssumption::TargetExists(
            AuthoritativeDependency::Occurrence(request.target_occurrence_id),
        )];
        if let Some(id) = parent {
            assumptions.push(ProposalAssumption::TargetExists(
                AuthoritativeDependency::Group(id),
            ));
        }
        let proposal = document
            .prepare_proposal_with_context(
                CommandBatch::new(commands),
                ProposalContext {
                    principal,
                    goal: ProposalGoal::SetOccurrenceParent(request.target_occurrence_id),
                    assumptions,
                    risk: ProposalRisk::Standard,
                    confirmation: ProposalConfirmation::ReviewRequired,
                    requested_budget: ProposalBudget::HOST_MAX,
                },
            )
            .map_err(|error| OccurrenceEditImpactError::InvalidCandidate(error.to_string()))?;
        let candidate = document
            .preview_batch(proposal.batch())
            .map_err(|error| OccurrenceEditImpactError::InvalidCandidate(error.to_string()))?;
        let candidate_target = candidate.occurrence(request.target_occurrence_id).ok_or(
            OccurrenceEditImpactError::OccurrenceNotFound(request.target_occurrence_id),
        )?;
        let world_transform_after = candidate
            .world_transform_for_occurrence(request.target_occurrence_id)
            .ok_or_else(|| {
                OccurrenceEditImpactError::InvalidCandidate(
                    "reparented occurrence has no valid world transform".to_owned(),
                )
            })?;
        if candidate_target.parent() != parent
            || candidate_target.transform() != local_transform_after
            || !transforms_nearly_equal(world_transform_after, world_transform_before)
            || !source.assembly_mates().eq(candidate.assembly_mates())
            || !source.collections().eq(candidate.collections())
            || !source.drawing_sheets().eq(candidate.drawing_sheets())
        {
            return Err(OccurrenceEditImpactError::InvalidCandidate(
                "occurrence reparent candidate did not preserve world-space dependencies"
                    .to_owned(),
            ));
        }
        for occurrence in source
            .occurrences()
            .filter(|occurrence| occurrence.id() != request.target_occurrence_id)
        {
            if candidate.occurrence(occurrence.id()) != Some(occurrence) {
                return Err(OccurrenceEditImpactError::InvalidCandidate(
                    "occurrence reparent candidate changed an unrelated occurrence".to_owned(),
                ));
            }
        }
        let collection_dependencies = source
            .collections()
            .filter_map(|collection| {
                let occurrence_ids = collection.occurrence_ids().collect::<Vec<_>>();
                occurrence_ids
                    .contains(&request.target_occurrence_id)
                    .then(|| OccurrenceCollectionDependencyImpact {
                        collection_id: collection.id(),
                        occurrence_ids_before: occurrence_ids.clone(),
                        occurrence_ids_after: occurrence_ids,
                    })
            })
            .collect();

        return Ok(OccurrenceEditImpactProjection {
            schema: OCCURRENCE_EDIT_IMPACT_SCHEMA_V1,
            source_revision: source.revision_id(),
            source_digest: source.canonical_digest(),
            candidate_digest: candidate.canonical_digest(),
            target_occurrence_id: request.target_occurrence_id,
            target_instance_path,
            edit: request.edit,
            parent_before,
            parent_after: parent,
            local_transform_before,
            local_transform_after: Some(local_transform_after),
            world_transform_before,
            world_transform_after: Some(world_transform_after),
            incident_mate_ids,
            collection_dependencies,
            drawing_dependencies: Vec::new(),
            proposal,
        });
    }

    let mut commands = incident_mate_ids
        .iter()
        .map(|id| CanonicalCommand::DeleteAssemblyMate { id: *id })
        .collect::<Vec<_>>();

    let mut collection_dependencies = Vec::new();
    for collection in source.collections() {
        let occurrence_ids_before = collection.occurrence_ids().collect::<Vec<_>>();
        if !occurrence_ids_before.contains(&request.target_occurrence_id) {
            continue;
        }
        let occurrence_ids_after = occurrence_ids_before
            .iter()
            .copied()
            .filter(|id| *id != request.target_occurrence_id)
            .collect::<Vec<_>>();
        commands.push(CanonicalCommand::SetCollectionOccurrences {
            id: collection.id(),
            occurrence_ids: occurrence_ids_after.clone(),
        });
        collection_dependencies.push(OccurrenceCollectionDependencyImpact {
            collection_id: collection.id(),
            occurrence_ids_before,
            occurrence_ids_after,
        });
    }

    let impacted_drawings = source
        .drawing_sheets()
        .filter_map(|sheet| match sheet.source() {
            DrawingSource::RigidAssembly { occurrence_ids } => occurrence_ids
                .contains(&request.target_occurrence_id)
                .then(|| {
                    (
                        sheet.clone(),
                        occurrence_ids.clone(),
                        occurrence_ids
                            .iter()
                            .copied()
                            .filter(|id| *id != request.target_occurrence_id)
                            .collect::<Vec<_>>(),
                    )
                }),
            DrawingSource::RigidAssemblyInstances { instance_paths } => instance_paths
                .iter()
                .any(|path| path.root_occurrence() == request.target_occurrence_id)
                .then(|| {
                    (
                        sheet.clone(),
                        instance_paths
                            .iter()
                            .map(InstancePath::root_occurrence)
                            .collect::<Vec<_>>(),
                        Vec::new(),
                    )
                }),
            DrawingSource::Definition(_) => None,
        })
        .collect::<Vec<_>>();
    let mut dependency_commands = commands.clone();
    dependency_commands.extend(
        impacted_drawings
            .iter()
            .map(|(sheet, _, _)| CanonicalCommand::DeleteDrawingSheet { id: sheet.id() }),
    );
    let dependency_candidate = document
        .preview_batch(&CommandBatch::new(dependency_commands))
        .map_err(|error| OccurrenceEditImpactError::InvalidCandidate(error.to_string()))?;
    let mut drawing_dependencies = Vec::new();
    for (sheet, occurrence_ids_before, occurrence_ids_after) in impacted_drawings {
        let sheet_id = sheet.id();
        let updated_source = DrawingSource::RigidAssembly {
            occurrence_ids: occurrence_ids_after.clone(),
        };
        let update = (!occurrence_ids_after.is_empty()
            && validate_source(&dependency_candidate, &updated_source).is_ok())
        .then(|| sheet.with_source(updated_source))
        .transpose()
        .map_err(|error| OccurrenceEditImpactError::InvalidCandidate(error.to_string()))?;
        let action = if let Some(updated_sheet) = update {
            commands.push(CanonicalCommand::UpdateDrawingSheet(updated_sheet));
            OccurrenceDrawingDependencyAction::UpdateRigidAssembly {
                occurrence_ids: occurrence_ids_after,
            }
        } else {
            commands.push(CanonicalCommand::DeleteDrawingSheet { id: sheet_id });
            OccurrenceDrawingDependencyAction::DeleteSheet
        };
        drawing_dependencies.push(OccurrenceDrawingDependencyImpact {
            sheet_id,
            occurrence_ids_before,
            action,
        });
    }
    commands.push(CanonicalCommand::DeleteOccurrence {
        id: request.target_occurrence_id,
    });

    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(commands),
            ProposalContext {
                principal,
                goal: ProposalGoal::DeleteOccurrence(request.target_occurrence_id),
                assumptions: vec![ProposalAssumption::TargetExists(
                    AuthoritativeDependency::Occurrence(request.target_occurrence_id),
                )],
                risk: ProposalRisk::Standard,
                confirmation: ProposalConfirmation::ReviewRequired,
                requested_budget: ProposalBudget::HOST_MAX,
            },
        )
        .map_err(|error| OccurrenceEditImpactError::InvalidCandidate(error.to_string()))?;
    let candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| OccurrenceEditImpactError::InvalidCandidate(error.to_string()))?;

    if candidate.occurrence(request.target_occurrence_id).is_some()
        || incident_mate_ids
            .iter()
            .any(|id| candidate.assembly_mate(*id).is_some())
        || collection_dependencies.iter().any(|impact| {
            candidate
                .collection(impact.collection_id)
                .is_none_or(|collection| {
                    collection.occurrence_ids().collect::<Vec<_>>() != impact.occurrence_ids_after
                })
        })
        || drawing_dependencies
            .iter()
            .any(|impact| match &impact.action {
                OccurrenceDrawingDependencyAction::UpdateRigidAssembly { occurrence_ids } => {
                    candidate
                        .drawing_sheet(impact.sheet_id)
                        .is_none_or(|sheet| {
                            sheet.source()
                                != &DrawingSource::RigidAssembly {
                                    occurrence_ids: occurrence_ids.clone(),
                                }
                        })
                }
                OccurrenceDrawingDependencyAction::DeleteSheet => {
                    candidate.drawing_sheet(impact.sheet_id).is_some()
                }
            })
    {
        return Err(OccurrenceEditImpactError::InvalidCandidate(
            "occurrence delete candidate did not apply its complete dependency closure".to_owned(),
        ));
    }
    for occurrence in source
        .occurrences()
        .filter(|occurrence| occurrence.id() != request.target_occurrence_id)
    {
        if candidate.occurrence(occurrence.id()) != Some(occurrence) {
            return Err(OccurrenceEditImpactError::InvalidCandidate(
                "occurrence delete candidate changed an unrelated occurrence".to_owned(),
            ));
        }
    }

    Ok(OccurrenceEditImpactProjection {
        schema: OCCURRENCE_EDIT_IMPACT_SCHEMA_V1,
        source_revision: source.revision_id(),
        source_digest: source.canonical_digest(),
        candidate_digest: candidate.canonical_digest(),
        target_occurrence_id: request.target_occurrence_id,
        target_instance_path,
        edit: request.edit,
        parent_before,
        parent_after: None,
        local_transform_before,
        local_transform_after: None,
        world_transform_before,
        world_transform_after: None,
        incident_mate_ids,
        collection_dependencies,
        drawing_dependencies,
        proposal,
    })
}

fn transforms_nearly_equal(left: Transform, right: Transform) -> bool {
    left.matrix()
        .iter()
        .zip(right.matrix())
        .all(|(left, right)| {
            let scale = left.abs().max(right.abs()).max(1.0);
            (left - right).abs() <= scale * ROUNDING
        })
}

#[derive(Clone, Debug, PartialEq)]
pub enum SharedDefinitionChange {
    ExactParameterEdit(BodyParameterEditRequest),
    BodyHistoryMutation(BodyHistoryMutationRequest),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedDefinitionChangeRequest {
    pub source_revision: u64,
    pub source_digest: String,
    pub change: SharedDefinitionChange,
}

impl SharedDefinitionChangeRequest {
    #[must_use]
    pub fn exact_parameter_edit(snapshot: &Snapshot, request: BodyParameterEditRequest) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            change: SharedDefinitionChange::ExactParameterEdit(request),
        }
    }

    #[must_use]
    pub fn body_history_mutation(snapshot: &Snapshot, request: BodyHistoryMutationRequest) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            change: SharedDefinitionChange::BodyHistoryMutation(request),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceForkChangeRequest {
    pub source_revision: u64,
    pub source_digest: String,
    pub selected_occurrence_id: OccurrenceId,
    pub new_definition_name: String,
    pub change: SharedDefinitionChange,
}

impl OccurrenceForkChangeRequest {
    #[must_use]
    pub fn exact_parameter_edit(
        snapshot: &Snapshot,
        selected_occurrence_id: OccurrenceId,
        new_definition_name: impl Into<String>,
        request: BodyParameterEditRequest,
    ) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            selected_occurrence_id,
            new_definition_name: new_definition_name.into(),
            change: SharedDefinitionChange::ExactParameterEdit(request),
        }
    }

    #[must_use]
    pub fn body_history_mutation(
        snapshot: &Snapshot,
        selected_occurrence_id: OccurrenceId,
        new_definition_name: impl Into<String>,
        request: BodyHistoryMutationRequest,
    ) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            selected_occurrence_id,
            new_definition_name: new_definition_name.into(),
            change: SharedDefinitionChange::BodyHistoryMutation(request),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OccurrenceForkBodyLineage {
    pub source_definition_id: DefinitionId,
    pub source_body_id: BodyId,
    pub fork_definition_id: DefinitionId,
    pub fork_body_id: BodyId,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OccurrenceForkFeatureLineage {
    pub source_feature_id: FeatureId,
    pub fork_feature_id: FeatureId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OccurrenceForkSubshapeLineage {
    pub source_definition_id: DefinitionId,
    pub source_profile_feature_id: FeatureId,
    pub source_producer_feature_id: FeatureId,
    pub source_lineage_digest: String,
    pub fork_definition_id: DefinitionId,
    pub fork_profile_feature_id: FeatureId,
    pub fork_producer_feature_id: FeatureId,
    pub fork_lineage_digest: String,
    pub semantic_role: String,
    pub source_element_id: String,
    pub expected_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurrenceForkMateReferenceImpact {
    pub mate_id: AssemblyMateId,
    pub occurrence_id: OccurrenceId,
    pub source_definition_id: DefinitionId,
    pub source_producer_feature_id: FeatureId,
    pub source_lineage_digest: String,
    pub fork_definition_id: DefinitionId,
    pub fork_producer_feature_id: FeatureId,
    pub fork_lineage_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceForkImpactProjection {
    pub source_revision: u64,
    pub source_digest: String,
    pub candidate_digest: String,
    pub selected_occurrence_id: OccurrenceId,
    pub selected_instance_path: InstancePath,
    pub source_definition_id: DefinitionId,
    pub fork_definition_id: DefinitionId,
    pub body_lineage: Vec<OccurrenceForkBodyLineage>,
    pub feature_lineage: Vec<OccurrenceForkFeatureLineage>,
    pub subshape_lineage: Vec<OccurrenceForkSubshapeLineage>,
    pub affected_fork_body_ids: Vec<BodyId>,
    pub affected_fork_feature_ids: Vec<FeatureId>,
    pub unchanged_source_body_ids: Vec<BodyId>,
    pub unchanged_sibling_occurrences: Vec<SharedChangeOccurrenceImpact>,
    pub unchanged_definition_ids: Vec<DefinitionId>,
    pub exact_jobs: Vec<SharedChangeExactJob>,
    pub mate_references: Vec<OccurrenceForkMateReferenceImpact>,
    pub collection_dependencies: Vec<OccurrenceCollectionDependencyImpact>,
    pub drawing_views: Vec<SharedChangeDrawingViewImpact>,
    pub exports: Vec<SharedChangeExportImpact>,
    pub proposal: Proposal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OccurrenceForkImpactError {
    Stale,
    OccurrenceNotFound(OccurrenceId),
    DefinitionNotReused(DefinitionId),
    Hidden(OccurrenceId),
    Failed(BodyId),
    Ambiguous(BodyId),
    Lost(AssemblyMateId),
    DependencyGraph(CanonicalError),
    CrossDefinition(DefinitionId, DefinitionId),
    Unsupported(DependencyBlocker),
}

impl fmt::Display for OccurrenceForkImpactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("occurrence fork change request is stale"),
            Self::OccurrenceNotFound(id) => write!(formatter, "occurrence {} was not found", id.0),
            Self::DefinitionNotReused(id) => write!(formatter, "definition {} is not reused", id.0),
            Self::Hidden(id) => write!(formatter, "occurrence {} is hidden", id.0),
            Self::Failed(body_id) => write!(
                formatter,
                "body {} has no last-valid exact result",
                body_id.0
            ),
            Self::Ambiguous(body_id) => {
                write!(formatter, "body {} has ambiguous exact results", body_id.0)
            }
            Self::Lost(mate_id) => write!(
                formatter,
                "assembly mate {} has a lost exact reference",
                mate_id.0
            ),
            Self::DependencyGraph(error) => {
                write!(formatter, "feature dependency graph is invalid: {error}")
            }
            Self::CrossDefinition(expected, actual) => write!(
                formatter,
                "occurrence definition {} does not match requested definition {}",
                expected.0, actual.0
            ),
            Self::Unsupported(blocker) => blocker.fmt(formatter),
        }
    }
}

impl std::error::Error for OccurrenceForkImpactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DependencyGraph(error) => Some(error),
            Self::Unsupported(blocker) => Some(blocker),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedChangeOccurrenceImpact {
    pub occurrence_id: OccurrenceId,
    pub instance_path: InstancePath,
    pub visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedChangeExactJob {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub producer_feature_id: FeatureId,
    pub canonical_input_digest: String,
    pub last_valid_result_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedChangeMateReferenceImpact {
    pub mate_id: AssemblyMateId,
    pub occurrence_id: OccurrenceId,
    pub definition_id: DefinitionId,
    pub producer_feature_id: FeatureId,
    pub lineage_digest: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SharedChangeDrawingViewImpact {
    pub sheet_id: DrawingSheetId,
    pub view: OrthographicViewKind,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SharedChangeExportFormat {
    Step,
    Stl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedChangeExportEligibility {
    PendingExactRecompute,
    CurrentExact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedChangeExportImpact {
    pub format: SharedChangeExportFormat,
    pub occurrence_paths: Vec<InstancePath>,
    pub eligibility: SharedChangeExportEligibility,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedChangeImpactProjection {
    pub source_revision: u64,
    pub source_digest: String,
    pub candidate_digest: String,
    pub definition_id: DefinitionId,
    pub affected_body_ids: Vec<BodyId>,
    pub affected_feature_ids: Vec<FeatureId>,
    pub unchanged_body_ids: Vec<BodyId>,
    pub unchanged_definition_ids: Vec<DefinitionId>,
    pub occurrences: Vec<SharedChangeOccurrenceImpact>,
    pub exact_jobs: Vec<SharedChangeExactJob>,
    pub mate_references: Vec<SharedChangeMateReferenceImpact>,
    pub drawing_views: Vec<SharedChangeDrawingViewImpact>,
    pub exports: Vec<SharedChangeExportImpact>,
    pub proposal: Proposal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedChangeOccurrenceRefresh {
    pub occurrence_id: OccurrenceId,
    pub instance_path: InstancePath,
    pub transform: Transform,
    pub visible: bool,
    pub result_fingerprint: String,
    pub subshape_lineage_digests: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedDefinitionPropagationReceipt {
    pub revision_id: u64,
    pub canonical_digest: String,
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub affected_feature_ids: Vec<FeatureId>,
    pub unchanged_body_ids: Vec<BodyId>,
    pub unchanged_definition_ids: Vec<DefinitionId>,
    pub occurrences: Vec<SharedChangeOccurrenceRefresh>,
    pub rebound_mate_ids: Vec<AssemblyMateId>,
    pub drawings: Vec<OrthographicDrawing>,
    pub exports: Vec<SharedChangeExportImpact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedChangePropagationError {
    Stale,
    InvalidImpact(String),
    Evaluation(String),
    ExactPublication(String),
    Dependency(DependencyBlocker),
    Commit(String),
}

impl fmt::Display for SharedChangePropagationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("shared-definition impact is stale"),
            Self::InvalidImpact(reason) => formatter.write_str(reason),
            Self::Evaluation(reason) => write!(formatter, "exact evaluation failed: {reason}"),
            Self::ExactPublication(reason) => {
                write!(formatter, "exact result publication failed: {reason}")
            }
            Self::Dependency(reason) => {
                write!(
                    formatter,
                    "shared-definition dependency refresh failed: {reason}"
                )
            }
            Self::Commit(reason) => write!(formatter, "shared-definition commit failed: {reason}"),
        }
    }
}

impl std::error::Error for SharedChangePropagationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Dependency(blocker) => Some(blocker),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedChangeImpactError {
    Stale,
    DefinitionNotReused(DefinitionId),
    Failed(BodyId),
    Ambiguous(BodyId),
    Lost(AssemblyMateId),
    DependencyGraph(CanonicalError),
    Unsupported(DependencyBlocker),
}

impl fmt::Display for SharedChangeImpactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("shared-definition change request is stale"),
            Self::DefinitionNotReused(id) => {
                write!(formatter, "definition {} is not reused", id.0)
            }
            Self::Failed(body_id) => write!(
                formatter,
                "body {} has no last-valid exact result",
                body_id.0
            ),
            Self::Ambiguous(body_id) => {
                write!(formatter, "body {} has ambiguous exact results", body_id.0)
            }
            Self::Lost(mate_id) => write!(
                formatter,
                "assembly mate {} has a lost exact reference",
                mate_id.0
            ),
            Self::DependencyGraph(error) => {
                write!(formatter, "feature dependency graph is invalid: {error}")
            }
            Self::Unsupported(blocker) => blocker.fmt(formatter),
        }
    }
}

impl std::error::Error for SharedChangeImpactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DependencyGraph(error) => Some(error),
            Self::Unsupported(blocker) => Some(blocker),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccurrenceForkCommitReceipt {
    pub revision_id: u64,
    pub canonical_digest: String,
    pub selected_occurrence: SharedChangeOccurrenceRefresh,
    pub source_definition_id: DefinitionId,
    pub fork_definition_id: DefinitionId,
    pub body_lineage: Vec<OccurrenceForkBodyLineage>,
    pub feature_lineage: Vec<OccurrenceForkFeatureLineage>,
    pub subshape_lineage: Vec<OccurrenceForkSubshapeLineage>,
    pub unaffected_sibling_occurrence_ids: Vec<OccurrenceId>,
    pub rebound_mate_ids: Vec<AssemblyMateId>,
    pub drawings: Vec<OrthographicDrawing>,
    pub exports: Vec<SharedChangeExportImpact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OccurrenceForkPropagationError {
    Stale,
    InvalidImpact(String),
    Evaluation(String),
    ExactPublication(String),
    Dependency(DependencyBlocker),
    Commit(String),
}

impl fmt::Display for OccurrenceForkPropagationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("occurrence fork impact is stale"),
            Self::InvalidImpact(reason) => formatter.write_str(reason),
            Self::Evaluation(reason) => write!(formatter, "fork exact evaluation failed: {reason}"),
            Self::ExactPublication(reason) => {
                write!(formatter, "fork exact result publication failed: {reason}")
            }
            Self::Dependency(reason) => {
                write!(
                    formatter,
                    "occurrence fork dependency refresh failed: {reason}"
                )
            }
            Self::Commit(reason) => write!(formatter, "occurrence fork commit failed: {reason}"),
        }
    }
}

impl std::error::Error for OccurrenceForkPropagationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Dependency(blocker) => Some(blocker),
            _ => None,
        }
    }
}

pub fn commit_occurrence_fork_change<E>(
    document: &mut DocumentStore,
    exact_results: &mut ExactResultRegistry,
    impact: &OccurrenceForkImpactProjection,
    mut evaluate: impl FnMut(&ExactBRepGraph) -> Result<Arc<ExactBodyPackage>, E>,
) -> Result<OccurrenceForkCommitReceipt, OccurrenceForkPropagationError>
where
    E: fmt::Display,
{
    let source = document.current();
    if source.revision_id() != impact.source_revision
        || source.canonical_digest() != impact.source_digest
        || impact.proposal.provenance_revision() != impact.source_revision
        || impact.proposal.provenance_digest() != impact.source_digest
    {
        return Err(OccurrenceForkPropagationError::Stale);
    }
    if !matches!(
        impact.proposal.confirmation(),
        ProposalConfirmation::ReviewRequired
    ) {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence-local Make Unique change was not reviewed".to_owned(),
        ));
    }

    let [body_id] = impact.affected_fork_body_ids.as_slice() else {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork must affect exactly one body branch".to_owned(),
        ));
    };
    let [job] = impact.exact_jobs.as_slice() else {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork must schedule exactly one exact body job".to_owned(),
        ));
    };
    if job.definition_id != impact.fork_definition_id || job.body_id != *body_id {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork exact job does not match the forked body".to_owned(),
        ));
    }

    let selected = source
        .occurrence(impact.selected_occurrence_id)
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "selected occurrence disappeared from the source snapshot".to_owned(),
            )
        })?;
    if selected.definition_id() != impact.source_definition_id {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "selected occurrence no longer uses the projected source definition".to_owned(),
        ));
    }
    let selected_world_transform = source
        .world_transform_for_occurrence(impact.selected_occurrence_id)
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "selected occurrence has no valid source world transform".to_owned(),
            )
        })?;
    let source_definition = source
        .definition(impact.source_definition_id)
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "projected source definition disappeared".to_owned(),
            )
        })?;
    let expected_definition_ids = source
        .definitions()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();
    if expected_definition_ids != impact.unchanged_definition_ids {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork definition-isolation evidence is incomplete".to_owned(),
        ));
    }
    let expected_body_lineage = source_definition
        .bodies()
        .map(|body| OccurrenceForkBodyLineage {
            source_definition_id: impact.source_definition_id,
            source_body_id: body.id(),
            fork_definition_id: impact.fork_definition_id,
            fork_body_id: body.id(),
        })
        .collect::<Vec<_>>();
    if expected_body_lineage != impact.body_lineage {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork body lineage is incomplete".to_owned(),
        ));
    }
    let expected_source_features = source_definition.feature_ids().to_vec();
    if impact
        .feature_lineage
        .iter()
        .map(|lineage| lineage.source_feature_id)
        .collect::<Vec<_>>()
        != expected_source_features
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork feature lineage is incomplete".to_owned(),
        ));
    }
    let mut expected_siblings = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.instance_path.is_root()
                && occurrence.definition_id == impact.source_definition_id
                && occurrence.occurrence_id != impact.selected_occurrence_id
        })
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path,
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();
    expected_siblings.sort_by(|left, right| left.instance_path.cmp(&right.instance_path));
    if expected_siblings != impact.unchanged_sibling_occurrences {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork sibling-isolation evidence is incomplete".to_owned(),
        ));
    }
    if preserved_collection_dependencies(&source, impact.selected_occurrence_id)
        != impact.collection_dependencies
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork collection dependency evidence is incomplete".to_owned(),
        ));
    }
    let mut expected_drawing_views = selected_sheets(&source, impact.selected_occurrence_id)
        .flat_map(|sheet| sheet_views(sheet.id()))
        .collect::<Vec<_>>();
    expected_drawing_views.sort_unstable();
    if expected_drawing_views != impact.drawing_views {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork drawing dependency evidence is incomplete".to_owned(),
        ));
    }

    let fork_feature_ids = impact
        .feature_lineage
        .iter()
        .map(|lineage| lineage.fork_feature_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut clone_count = 0;
    let mut edit_count = 0;
    for command in impact.proposal.batch().commands() {
        match command {
            CanonicalCommand::CloneDefinitionAndRepoint(_) => clone_count += 1,
            CanonicalCommand::SetFeatureDimension { id, .. }
            | CanonicalCommand::SetSketchConstraintDimension { id, .. }
                if fork_feature_ids.contains(id) =>
            {
                edit_count += 1;
            }
            CanonicalCommand::SetFeatureParameter { target, .. }
                if fork_feature_ids.contains(&target.feature_id) =>
            {
                edit_count += 1;
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id: command_body_id,
                suppressed_feature_ids,
            } if *definition_id == impact.fork_definition_id
                && *command_body_id == *body_id
                && suppressed_feature_ids
                    .iter()
                    .all(|feature_id| fork_feature_ids.contains(feature_id)) =>
            {
                edit_count += 1;
            }
            CanonicalCommand::RebindAssemblyMate(rebound) => {
                let source_mate = source.assembly_mate(rebound.id()).ok_or_else(|| {
                    OccurrenceForkPropagationError::InvalidImpact(
                        "occurrence fork proposal introduced an unrelated mate".to_owned(),
                    )
                })?;
                if source_mate.kind() != rebound.kind() {
                    return Err(OccurrenceForkPropagationError::InvalidImpact(
                        "occurrence fork proposal changed a mate kind".to_owned(),
                    ));
                }
                for (before, after) in [source_mate.endpoint_a(), source_mate.endpoint_b()]
                    .into_iter()
                    .zip([rebound.endpoint_a(), rebound.endpoint_b()])
                {
                    if before.occurrence_id() != after.occurrence_id()
                        || (before.occurrence_id() != impact.selected_occurrence_id
                            && before != after)
                        || (before.occurrence_id() == impact.selected_occurrence_id
                            && (after.health() != AssemblyReferenceHealth::Resolved
                                || after.reference().definition_id != impact.fork_definition_id
                                || !fork_feature_ids
                                    .contains(&after.reference().profile_feature_id)
                                || !fork_feature_ids
                                    .contains(&after.reference().producer_feature_id)))
                    {
                        return Err(OccurrenceForkPropagationError::InvalidImpact(
                            "occurrence fork proposal changed an unrelated mate endpoint"
                                .to_owned(),
                        ));
                    }
                }
            }
            _ => {
                return Err(OccurrenceForkPropagationError::InvalidImpact(
                    "occurrence fork proposal contains an unrelated canonical command".to_owned(),
                ));
            }
        }
    }
    if clone_count != 1 || edit_count == 0 {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork proposal must contain one clone and a supported fork edit".to_owned(),
        ));
    }

    let candidate = document
        .preview_batch(impact.proposal.batch())
        .map_err(|error| OccurrenceForkPropagationError::InvalidImpact(error.to_string()))?;
    if candidate.canonical_digest() != impact.candidate_digest {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork candidate digest changed".to_owned(),
        ));
    }
    if candidate.definition(impact.source_definition_id) != Some(source_definition) {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork candidate changed the source definition".to_owned(),
        ));
    }
    let fork_definition = candidate
        .definition(impact.fork_definition_id)
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "occurrence fork candidate did not create the fork definition".to_owned(),
            )
        })?;
    if fork_definition.feature_ids()
        != impact
            .feature_lineage
            .iter()
            .map(|lineage| lineage.fork_feature_id)
            .collect::<Vec<_>>()
            .as_slice()
        || fork_definition
            .bodies()
            .map(|body| body.id())
            .collect::<Vec<_>>()
            != impact
                .body_lineage
                .iter()
                .map(|lineage| lineage.fork_body_id)
                .collect::<Vec<_>>()
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork candidate does not match the projected branch lineage".to_owned(),
        ));
    }
    let candidate_selected = candidate
        .occurrence(impact.selected_occurrence_id)
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "occurrence fork candidate lost the selected occurrence".to_owned(),
            )
        })?;
    if candidate_selected.id() != selected.id()
        || candidate_selected.definition_id() != impact.fork_definition_id
        || candidate_selected.name() != selected.name()
        || candidate_selected.transform() != selected.transform()
        || candidate_selected.parent() != selected.parent()
        || candidate_selected.tag() != selected.tag()
        || candidate_selected.visible() != selected.visible()
        || candidate
            .world_transform_for_occurrence(impact.selected_occurrence_id)
            .is_none_or(|transform| !transforms_nearly_equal(transform, selected_world_transform))
        || !source.collections().eq(candidate.collections())
        || !source.drawing_sheets().eq(candidate.drawing_sheets())
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork candidate changed identity, world placement, or preserved dependencies"
                .to_owned(),
        ));
    }
    for occurrence in source
        .occurrences()
        .filter(|occurrence| occurrence.id() != impact.selected_occurrence_id)
    {
        if candidate.occurrence(occurrence.id()) != Some(occurrence) {
            return Err(OccurrenceForkPropagationError::InvalidImpact(
                "occurrence fork candidate changed an unrelated occurrence".to_owned(),
            ));
        }
    }
    for definition_id in expected_definition_ids {
        if definition_id != impact.source_definition_id
            && source.definition(definition_id) != candidate.definition(definition_id)
        {
            return Err(OccurrenceForkPropagationError::InvalidImpact(
                "occurrence fork candidate changed an unrelated definition".to_owned(),
            ));
        }
    }

    let previous = exact_results
        .get_body(&source, impact.source_definition_id, *body_id)
        .map_err(|error| OccurrenceForkPropagationError::ExactPublication(error.to_string()))?
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "last-valid source exact body result is unavailable".to_owned(),
            )
        })?;
    if previous.result_key().result_fingerprint != job.last_valid_result_fingerprint {
        return Err(OccurrenceForkPropagationError::Stale);
    }
    let graph = body_exact_graph(&candidate, impact.fork_definition_id, *body_id)
        .map_err(|error| OccurrenceForkPropagationError::InvalidImpact(error.to_string()))?;
    if FeatureId(graph.producer_feature_id) != job.producer_feature_id
        || graph.canonical_input_digest != job.canonical_input_digest
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork exact job changed".to_owned(),
        ));
    }

    let package = evaluate(&graph)
        .map_err(|error| OccurrenceForkPropagationError::Evaluation(error.to_string()))?;
    let staged_results = ExactResultRegistry::publish_body_results(
        &candidate,
        exact_results,
        [Arc::clone(&package)],
    )
    .map_err(|error| OccurrenceForkPropagationError::ExactPublication(error.to_string()))?;
    let staged_fork_package = staged_results
        .get_body(&candidate, impact.fork_definition_id, *body_id)
        .map_err(|error| OccurrenceForkPropagationError::ExactPublication(error.to_string()))?
        .ok_or_else(|| {
            OccurrenceForkPropagationError::ExactPublication(
                "fork render/pick package was not published".to_owned(),
            )
        })?;
    let source_package = staged_results
        .get_body(&candidate, impact.source_definition_id, *body_id)
        .map_err(|error| OccurrenceForkPropagationError::ExactPublication(error.to_string()))?
        .ok_or_else(|| {
            OccurrenceForkPropagationError::ExactPublication(
                "source last-valid render/pick package was not preserved".to_owned(),
            )
        })?;
    if source_package.result_key().result_fingerprint != previous.result_key().result_fingerprint {
        return Err(OccurrenceForkPropagationError::ExactPublication(
            "source render/pick package changed during occurrence fork".to_owned(),
        ));
    }
    if staged_fork_package.references().iter().any(|reference| {
        !impact.subshape_lineage.iter().any(|lineage| {
            lineage.fork_definition_id == reference.definition_id
                && lineage.fork_profile_feature_id == reference.profile_feature_id
                && lineage.fork_producer_feature_id == reference.producer_feature_id
                && lineage.fork_lineage_digest == reference.lineage_digest
                && lineage.semantic_role == reference.semantic_role
                && lineage.source_element_id == reference.source_element_id
                && lineage.expected_type == reference.expected_type
        })
    }) {
        return Err(OccurrenceForkPropagationError::ExactPublication(
            "fork render/pick package does not follow projected subshape lineage".to_owned(),
        ));
    }

    let direct_mate_ids = impact
        .mate_references
        .iter()
        .map(|reference| reference.mate_id)
        .collect::<std::collections::BTreeSet<_>>();
    let affected_mate_ids = dependent_mate_component(&candidate, &direct_mate_ids);
    let sibling_ids = impact
        .unchanged_sibling_occurrences
        .iter()
        .map(|occurrence| occurrence.occurrence_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut commands = impact.proposal.batch().commands().to_vec();
    let mut rebound_mate_ids = Vec::new();
    if !affected_mate_ids.is_empty() {
        let recomputed = recompute_rigid_assembly_mates_from_snapshot(
            &candidate,
            &staged_results,
            AssemblySolverPolicy::default(),
            &affected_mate_ids,
        )
        .map_err(|error| {
            OccurrenceForkPropagationError::Dependency(DependencyBlocker::AssemblyRecompute(error))
        })?;
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::AssemblyNotSolved(recomputed.status()),
            ));
        }
        for mate in recomputed.mates() {
            if !direct_mate_ids.contains(&mate.id()) {
                continue;
            }
            let current = candidate.assembly_mate(mate.id()).ok_or_else(|| {
                OccurrenceForkPropagationError::Dependency(DependencyBlocker::Mate {
                    mate: mate.id(),
                    problem: MateProblem::Missing,
                })
            })?;
            let selected_endpoint =
                |before: &AssemblyMateEndpoint, after: &AssemblyMateEndpoint| {
                    if before.occurrence_id() == impact.selected_occurrence_id {
                        after.clone()
                    } else {
                        before.clone()
                    }
                };
            let local_rebind = AssemblyMate::new(
                mate.id(),
                selected_endpoint(current.endpoint_a(), mate.endpoint_a()),
                selected_endpoint(current.endpoint_b(), mate.endpoint_b()),
                current.kind(),
            );
            if &local_rebind != current {
                rebound_mate_ids.push(mate.id());
                commands.push(CanonicalCommand::RebindAssemblyMate(local_rebind));
            }
        }
        let transforms = recomputed
            .solve()
            .into_iter()
            .flat_map(|solve| solve.occurrences())
            .filter(|occurrence| !occurrence.grounded())
            .filter_map(|occurrence| {
                candidate
                    .occurrence(occurrence.occurrence_id())
                    .filter(|current| current.transform() != occurrence.transform())
                    .map(|_| (occurrence.occurrence_id(), occurrence.transform()))
            })
            .collect::<Vec<_>>();
        if transforms
            .iter()
            .any(|(occurrence_id, _)| *occurrence_id == impact.selected_occurrence_id)
        {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::SolveMovesSelected,
            ));
        }
        if transforms
            .iter()
            .any(|(occurrence_id, _)| sibling_ids.contains(occurrence_id))
        {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::SolveMovesSibling,
            ));
        }
        if !transforms.is_empty() {
            commands.push(CanonicalCommand::ApplyAssemblySolve {
                source_revision: impact.source_revision,
                source_digest: impact.source_digest.clone(),
                transforms,
                instance_transforms: Vec::new(),
            });
        }
    }
    rebound_mate_ids.sort_unstable();

    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(commands),
            ProposalContext {
                principal: impact.proposal.principal(),
                goal: impact.proposal.goal(),
                assumptions: impact.proposal.assumptions().to_vec(),
                risk: impact.proposal.risk(),
                confirmation: impact.proposal.confirmation().clone(),
                requested_budget: impact.proposal.requested_budget(),
            },
        )
        .map_err(|error| {
            OccurrenceForkPropagationError::Dependency(DependencyBlocker::Proposal(error))
        })?;
    let final_candidate = document.preview_batch(proposal.batch()).map_err(|error| {
        OccurrenceForkPropagationError::Dependency(DependencyBlocker::Staging(error))
    })?;
    let final_results = ExactResultRegistry::carried_forward(&final_candidate, &staged_results);
    if final_candidate
        .world_transform_for_occurrence(impact.selected_occurrence_id)
        .is_none_or(|transform| !transforms_nearly_equal(transform, selected_world_transform))
        || !source.collections().eq(final_candidate.collections())
        || !source.drawing_sheets().eq(final_candidate.drawing_sheets())
    {
        return Err(OccurrenceForkPropagationError::Dependency(
            DependencyBlocker::PlacementNotPreserved,
        ));
    }
    validate_occurrence_fork_mates(&source, &final_candidate, &final_results, impact)?;
    let drawings = refresh_occurrence_fork_drawings(&final_candidate, &final_results, impact)?;
    let exports =
        refresh_occurrence_fork_exports(&final_candidate, &final_results, impact, *body_id)?;
    let fork_package = final_results
        .get_body(&final_candidate, impact.fork_definition_id, *body_id)
        .map_err(|error| OccurrenceForkPropagationError::ExactPublication(error.to_string()))?
        .ok_or_else(|| {
            OccurrenceForkPropagationError::ExactPublication(
                "fork exact result did not survive dependency staging".to_owned(),
            )
        })?;
    let mut lineage_digests = fork_package
        .references()
        .iter()
        .map(|reference| reference.lineage_digest.clone())
        .collect::<Vec<_>>();
    lineage_digests.sort();
    let result_fingerprint = fork_package.result_key().result_fingerprint;
    let selected_scene = final_candidate
        .scene_query()
        .into_iter()
        .find(|occurrence| occurrence.instance_path == impact.selected_instance_path)
        .ok_or_else(|| {
            OccurrenceForkPropagationError::InvalidImpact(
                "selected occurrence render/pick projection disappeared".to_owned(),
            )
        })?;

    let revision = document
        .commit_proposal(&proposal)
        .map_err(|error: ProposalCommitError| {
            OccurrenceForkPropagationError::Commit(error.to_string())
        })?;
    *exact_results = final_results;

    Ok(OccurrenceForkCommitReceipt {
        revision_id: revision.id(),
        canonical_digest: revision.snapshot().canonical_digest(),
        selected_occurrence: SharedChangeOccurrenceRefresh {
            occurrence_id: impact.selected_occurrence_id,
            instance_path: selected_scene.instance_path,
            transform: selected_scene.transform,
            visible: selected_scene.visible,
            result_fingerprint,
            subshape_lineage_digests: lineage_digests,
        },
        source_definition_id: impact.source_definition_id,
        fork_definition_id: impact.fork_definition_id,
        body_lineage: impact.body_lineage.clone(),
        feature_lineage: impact.feature_lineage.clone(),
        subshape_lineage: impact.subshape_lineage.clone(),
        unaffected_sibling_occurrence_ids: impact
            .unchanged_sibling_occurrences
            .iter()
            .map(|occurrence| occurrence.occurrence_id)
            .collect(),
        rebound_mate_ids,
        drawings,
        exports,
    })
}

fn validate_occurrence_fork_mates(
    source: &Snapshot,
    candidate: &Snapshot,
    exact_results: &ExactResultRegistry,
    impact: &OccurrenceForkImpactProjection,
) -> Result<(), OccurrenceForkPropagationError> {
    let direct_mate_ids = impact
        .mate_references
        .iter()
        .map(|reference| reference.mate_id)
        .collect::<std::collections::BTreeSet<_>>();
    if source.assembly_mates().count() != candidate.assembly_mates().count() {
        return Err(OccurrenceForkPropagationError::Dependency(
            DependencyBlocker::MateSetChanged,
        ));
    }
    for source_mate in source.assembly_mates() {
        let candidate_mate = candidate.assembly_mate(source_mate.id()).ok_or_else(|| {
            OccurrenceForkPropagationError::Dependency(DependencyBlocker::Mate {
                mate: source_mate.id(),
                problem: MateProblem::Missing,
            })
        })?;
        if !direct_mate_ids.contains(&source_mate.id()) {
            if candidate_mate != source_mate {
                return Err(OccurrenceForkPropagationError::Dependency(
                    DependencyBlocker::Mate {
                        mate: source_mate.id(),
                        problem: MateProblem::UnrelatedChanged,
                    },
                ));
            }
            continue;
        }
        if candidate_mate.kind() != source_mate.kind() {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::Mate {
                    mate: source_mate.id(),
                    problem: MateProblem::KindChanged,
                },
            ));
        }
        for (before, after) in [source_mate.endpoint_a(), source_mate.endpoint_b()]
            .into_iter()
            .zip([candidate_mate.endpoint_a(), candidate_mate.endpoint_b()])
        {
            if before.occurrence_id() != impact.selected_occurrence_id && before != after {
                return Err(OccurrenceForkPropagationError::Dependency(
                    DependencyBlocker::Mate {
                        mate: source_mate.id(),
                        problem: MateProblem::OtherEndpointChanged,
                    },
                ));
            }
        }
    }

    for expected in &impact.mate_references {
        let mate = candidate.assembly_mate(expected.mate_id).ok_or({
            OccurrenceForkPropagationError::Dependency(DependencyBlocker::Mate {
                mate: expected.mate_id,
                problem: MateProblem::Missing,
            })
        })?;
        let endpoint = [mate.endpoint_a(), mate.endpoint_b()]
            .into_iter()
            .find(|endpoint| {
                endpoint.occurrence_id() == expected.occurrence_id
                    && endpoint.reference().lineage_digest == expected.fork_lineage_digest
            })
            .ok_or({
                OccurrenceForkPropagationError::Dependency(DependencyBlocker::Mate {
                    mate: expected.mate_id,
                    problem: MateProblem::ForkLineageLost,
                })
            })?;
        if endpoint.health() != AssemblyReferenceHealth::Resolved
            || endpoint.reference().definition_id != expected.fork_definition_id
            || endpoint.reference().producer_feature_id != expected.fork_producer_feature_id
        {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::Mate {
                    mate: expected.mate_id,
                    problem: MateProblem::NotResolvedToFork,
                },
            ));
        }
        match exact_results.resolve_reference(candidate, endpoint.reference()) {
            ExactReferenceResolution::Resolved { reference }
                if reference.as_ref() == endpoint.reference() => {}
            resolution => {
                return Err(OccurrenceForkPropagationError::Dependency(
                    DependencyBlocker::Mate {
                        mate: expected.mate_id,
                        problem: MateProblem::NotUniquelyCurrent(resolution),
                    },
                ));
            }
        }
    }
    Ok(())
}

fn refresh_occurrence_fork_drawings(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    impact: &OccurrenceForkImpactProjection,
) -> Result<Vec<OrthographicDrawing>, OccurrenceForkPropagationError> {
    let mut drawings = Vec::<OrthographicDrawing>::new();
    for affected in &impact.drawing_views {
        if drawings
            .last()
            .is_some_and(|drawing| drawing.sheet_id == affected.sheet_id)
        {
            continue;
        }
        let sheet = snapshot.drawing_sheet(affected.sheet_id).ok_or({
            OccurrenceForkPropagationError::Dependency(DependencyBlocker::Sheet {
                sheet: affected.sheet_id,
                problem: SheetProblem::Missing,
            })
        })?;
        let drawing =
            project_orthographic_drawing(snapshot, exact_results, sheet).map_err(|error| {
                OccurrenceForkPropagationError::Dependency(DependencyBlocker::Sheet {
                    sheet: affected.sheet_id,
                    problem: SheetProblem::Projection(error),
                })
            })?;
        if !drawing.is_current(snapshot) {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::Sheet {
                    sheet: affected.sheet_id,
                    problem: SheetProblem::NotRefreshed,
                },
            ));
        }
        drawings.push(drawing);
    }
    Ok(drawings)
}

fn refresh_occurrence_fork_exports(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    impact: &OccurrenceForkImpactProjection,
    body_id: BodyId,
) -> Result<Vec<SharedChangeExportImpact>, OccurrenceForkPropagationError> {
    let scene = snapshot.scene_query();
    let mut refreshed = Vec::with_capacity(impact.exports.len());
    for affected in &impact.exports {
        if affected.occurrence_paths.as_slice() != [impact.selected_instance_path.clone()] {
            return Err(OccurrenceForkPropagationError::Dependency(
                DependencyBlocker::ExportOutsideSelection,
            ));
        }
        let mut bodies = Vec::with_capacity(affected.occurrence_paths.len());
        for path in &affected.occurrence_paths {
            let occurrence = scene
                .iter()
                .find(|occurrence| occurrence.instance_path == *path && occurrence.visible)
                .ok_or_else(|| {
                    OccurrenceForkPropagationError::Dependency(DependencyBlocker::Export {
                        path: path.clone(),
                        problem: ExportProblem::NotVisible,
                    })
                })?;
            if occurrence.definition_id != impact.fork_definition_id {
                return Err(OccurrenceForkPropagationError::Dependency(
                    DependencyBlocker::Export {
                        path: path.clone(),
                        problem: ExportProblem::NotForkDefinition,
                    },
                ));
            }
            let package = exact_results
                .get_body(snapshot, occurrence.definition_id, body_id)
                .map_err(|error| {
                    OccurrenceForkPropagationError::Dependency(DependencyBlocker::Exact(error))
                })?
                .ok_or_else(|| {
                    OccurrenceForkPropagationError::Dependency(DependencyBlocker::Export {
                        path: path.clone(),
                        problem: ExportProblem::NoExactBody,
                    })
                })?;
            producer_exact_graph(
                snapshot,
                occurrence.definition_id,
                package.producer_feature_id(),
            )
            .map_err(|error| {
                OccurrenceForkPropagationError::Dependency(DependencyBlocker::Exact(error))
            })?;
            bodies.push((package.as_ref(), occurrence.transform));
        }
        if affected.format == SharedChangeExportFormat::Stl {
            exact_model_stl_export(snapshot, &bodies).map_err(|error| {
                OccurrenceForkPropagationError::Dependency(DependencyBlocker::Exact(error))
            })?;
        }
        refreshed.push(SharedChangeExportImpact {
            format: affected.format,
            occurrence_paths: affected.occurrence_paths.clone(),
            eligibility: SharedChangeExportEligibility::CurrentExact,
        });
    }
    Ok(refreshed)
}

pub fn commit_shared_definition_change<E>(
    document: &mut DocumentStore,
    exact_results: &mut ExactResultRegistry,
    impact: &SharedChangeImpactProjection,
    mut evaluate: impl FnMut(&ExactBRepGraph) -> Result<Arc<ExactBodyPackage>, E>,
) -> Result<SharedDefinitionPropagationReceipt, SharedChangePropagationError>
where
    E: fmt::Display,
{
    let source = document.current();
    if source.revision_id() != impact.source_revision
        || source.canonical_digest() != impact.source_digest
        || impact.proposal.provenance_revision() != impact.source_revision
        || impact.proposal.provenance_digest() != impact.source_digest
    {
        return Err(SharedChangePropagationError::Stale);
    }
    if !matches!(
        impact.proposal.confirmation(),
        ProposalConfirmation::ReviewRequired
    ) {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition change was not reviewed".to_owned(),
        ));
    }

    if impact.affected_body_ids.is_empty()
        || impact
            .affected_body_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition affected bodies are empty or non-canonical".to_owned(),
        ));
    }
    let [job] = impact.exact_jobs.as_slice() else {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition change must schedule exactly one terminal exact body job".to_owned(),
        ));
    };
    let body_id = job.body_id;
    if job.definition_id != impact.definition_id || !impact.affected_body_ids.contains(&body_id) {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition exact job is outside the affected body closure".to_owned(),
        ));
    }

    let mut source_occurrences = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.definition_id == impact.definition_id)
        .collect::<Vec<_>>();
    source_occurrences.sort_by(|left, right| left.instance_path.cmp(&right.instance_path));
    let projected_occurrences = source_occurrences
        .iter()
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path.clone(),
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();
    if projected_occurrences != impact.occurrences || source_occurrences.len() < 2 {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition occurrence impact is incomplete".to_owned(),
        ));
    }
    let unchanged_definition_ids = source
        .definitions()
        .filter_map(|definition| {
            (definition.id() != impact.definition_id).then_some(definition.id())
        })
        .collect::<Vec<_>>();
    if unchanged_definition_ids != impact.unchanged_definition_ids {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition isolation evidence is incomplete".to_owned(),
        ));
    }

    let previous = exact_results
        .get_body(&source, impact.definition_id, body_id)
        .map_err(|error| SharedChangePropagationError::ExactPublication(error.to_string()))?
        .ok_or_else(|| {
            SharedChangePropagationError::InvalidImpact(
                "last-valid exact body result is unavailable".to_owned(),
            )
        })?;
    if previous.result_key().result_fingerprint != job.last_valid_result_fingerprint {
        return Err(SharedChangePropagationError::Stale);
    }

    let candidate = document
        .preview_dependency_staging_batch(impact.proposal.batch())
        .map_err(|error| SharedChangePropagationError::InvalidImpact(error.to_string()))?;
    if candidate.canonical_digest() != impact.candidate_digest {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition candidate digest changed".to_owned(),
        ));
    }
    let graph = body_exact_graph(&candidate, impact.definition_id, body_id)
        .map_err(|error| SharedChangePropagationError::InvalidImpact(error.to_string()))?;
    if FeatureId(graph.producer_feature_id) != job.producer_feature_id
        || graph.canonical_input_digest != job.canonical_input_digest
    {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition exact job changed".to_owned(),
        ));
    }

    let package = evaluate(&graph)
        .map_err(|error| SharedChangePropagationError::Evaluation(error.to_string()))?;
    let staged_results = ExactResultRegistry::publish_body_results(
        &candidate,
        exact_results,
        [Arc::clone(&package)],
    )
    .map_err(|error| SharedChangePropagationError::ExactPublication(error.to_string()))?;
    let producer_transition = previous.producer_feature_id() != package.producer_feature_id();
    validate_stable_lineage(previous, &package, producer_transition)?;

    let direct_mate_ids = impact
        .mate_references
        .iter()
        .map(|reference| reference.mate_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut commands = impact.proposal.batch().commands().to_vec();
    if producer_transition {
        commands.extend(rebind_producer_transition_mates(
            &candidate,
            &package,
            impact.definition_id,
            &direct_mate_ids,
        )?);
    }
    let dependency_candidate = document
        .preview_batch(&CommandBatch::new(commands.clone()))
        .map_err(|error| {
            SharedChangePropagationError::Dependency(DependencyBlocker::Staging(error))
        })?;
    let dependency_results =
        ExactResultRegistry::carried_forward(&dependency_candidate, &staged_results);
    let affected_mate_ids = dependent_mate_component(&dependency_candidate, &direct_mate_ids);
    let mut rebound_mate_ids = Vec::new();
    if !affected_mate_ids.is_empty() {
        let recomputed = recompute_rigid_assembly_mates_from_snapshot(
            &dependency_candidate,
            &dependency_results,
            AssemblySolverPolicy::default(),
            &affected_mate_ids,
        )
        .map_err(|error| {
            SharedChangePropagationError::Dependency(DependencyBlocker::AssemblyRecompute(error))
        })?;
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            return Err(SharedChangePropagationError::Dependency(
                DependencyBlocker::AssemblyNotSolved(recomputed.status()),
            ));
        }
        for mate in recomputed.mates() {
            if affected_mate_ids.contains(&mate.id())
                && dependency_candidate.assembly_mate(mate.id()) != Some(mate)
            {
                rebound_mate_ids.push(mate.id());
                commands.push(CanonicalCommand::RebindAssemblyMate(mate.clone()));
            }
        }
        let transforms = recomputed
            .solve()
            .into_iter()
            .flat_map(|solve| solve.occurrences())
            .filter(|occurrence| !occurrence.grounded())
            .filter_map(|occurrence| {
                dependency_candidate
                    .occurrence(occurrence.occurrence_id())
                    .filter(|current| current.transform() != occurrence.transform())
                    .map(|_| (occurrence.occurrence_id(), occurrence.transform()))
            })
            .collect::<Vec<_>>();
        if !transforms.is_empty() {
            commands.push(CanonicalCommand::ApplyAssemblySolve {
                source_revision: impact.source_revision,
                source_digest: impact.source_digest.clone(),
                transforms,
                instance_transforms: Vec::new(),
            });
        }
    }

    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(commands),
            ProposalContext {
                principal: impact.proposal.principal(),
                goal: impact.proposal.goal(),
                assumptions: impact.proposal.assumptions().to_vec(),
                risk: impact.proposal.risk(),
                confirmation: impact.proposal.confirmation().clone(),
                requested_budget: impact.proposal.requested_budget(),
            },
        )
        .map_err(|error| {
            SharedChangePropagationError::Dependency(DependencyBlocker::Proposal(error))
        })?;
    let final_candidate = document.preview_batch(proposal.batch()).map_err(|error| {
        SharedChangePropagationError::Dependency(DependencyBlocker::Staging(error))
    })?;
    let final_results = ExactResultRegistry::carried_forward(&final_candidate, &staged_results);
    let final_package = final_results
        .get_body(&final_candidate, impact.definition_id, body_id)
        .map_err(|error| SharedChangePropagationError::ExactPublication(error.to_string()))?
        .ok_or_else(|| {
            SharedChangePropagationError::ExactPublication(
                "affected exact result did not survive dependency staging".to_owned(),
            )
        })?;

    validate_rebound_mates(
        &final_candidate,
        &final_results,
        impact,
        producer_transition,
        job.producer_feature_id,
    )?;
    let drawings = refresh_drawings(&final_candidate, &final_results, impact)?;
    let exports = refresh_export_eligibility(&final_candidate, &final_results, impact, body_id)?;

    let mut lineage_digests = final_package
        .references()
        .iter()
        .map(|reference| reference.lineage_digest.clone())
        .collect::<Vec<_>>();
    lineage_digests.sort();
    let result_fingerprint = final_package.result_key().result_fingerprint;
    let occurrences = final_candidate
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.definition_id == impact.definition_id)
        .map(|occurrence| SharedChangeOccurrenceRefresh {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path,
            transform: occurrence.transform,
            visible: occurrence.visible,
            result_fingerprint: result_fingerprint.clone(),
            subshape_lineage_digests: lineage_digests.clone(),
        })
        .collect();

    let revision = document
        .commit_proposal(&proposal)
        .map_err(|error: ProposalCommitError| {
            SharedChangePropagationError::Commit(error.to_string())
        })?;
    *exact_results = final_results;

    Ok(SharedDefinitionPropagationReceipt {
        revision_id: revision.id(),
        canonical_digest: revision.snapshot().canonical_digest(),
        definition_id: impact.definition_id,
        body_id,
        affected_feature_ids: impact.affected_feature_ids.clone(),
        unchanged_body_ids: impact.unchanged_body_ids.clone(),
        unchanged_definition_ids: impact.unchanged_definition_ids.clone(),
        occurrences,
        rebound_mate_ids,
        drawings,
        exports,
    })
}

fn rebind_producer_transition_mates(
    snapshot: &Snapshot,
    package: &ExactBodyPackage,
    definition_id: DefinitionId,
    mate_ids: &std::collections::BTreeSet<AssemblyMateId>,
) -> Result<Vec<CanonicalCommand>, SharedChangePropagationError> {
    let mut commands = Vec::new();
    for mate in snapshot
        .assembly_mates()
        .filter(|mate| mate_ids.contains(&mate.id()))
    {
        let rebind = |endpoint: &AssemblyMateEndpoint| {
            if endpoint.reference().definition_id != definition_id {
                return Ok(endpoint.clone());
            }
            let mut matches = package.references().iter().filter(|reference| {
                reference.semantic_role == endpoint.reference().semantic_role
                    && reference.source_element_id == endpoint.reference().source_element_id
                    && reference.expected_type == endpoint.reference().expected_type
            });
            let reference = matches.next().ok_or_else(|| {
                SharedChangePropagationError::Dependency(DependencyBlocker::Mate {
                    mate: mate.id(),
                    problem: MateProblem::RoleLost {
                        role: endpoint.reference().semantic_role.clone(),
                    },
                })
            })?;
            if matches.next().is_some() {
                return Err(SharedChangePropagationError::Dependency(
                    DependencyBlocker::Mate {
                        mate: mate.id(),
                        problem: MateProblem::RoleAmbiguous {
                            role: endpoint.reference().semantic_role.clone(),
                        },
                    },
                ));
            }
            Ok(AssemblyMateEndpoint::resolved(
                endpoint.occurrence_id(),
                reference.clone(),
            ))
        };
        let rebound = AssemblyMate::new(
            mate.id(),
            rebind(mate.endpoint_a())?,
            rebind(mate.endpoint_b())?,
            mate.kind(),
        );
        if &rebound != mate {
            commands.push(CanonicalCommand::RebindAssemblyMate(rebound));
        }
    }
    Ok(commands)
}

fn dependent_mate_component(
    snapshot: &Snapshot,
    direct_mate_ids: &std::collections::BTreeSet<AssemblyMateId>,
) -> std::collections::BTreeSet<AssemblyMateId> {
    let mut mate_ids = direct_mate_ids.clone();
    let mut occurrence_ids = std::collections::BTreeSet::new();
    for mate in snapshot
        .assembly_mates()
        .filter(|mate| mate_ids.contains(&mate.id()))
    {
        occurrence_ids.insert(mate.endpoint_a().occurrence_id());
        occurrence_ids.insert(mate.endpoint_b().occurrence_id());
    }
    loop {
        let mut changed = false;
        for mate in snapshot.assembly_mates() {
            if occurrence_ids.contains(&mate.endpoint_a().occurrence_id())
                || occurrence_ids.contains(&mate.endpoint_b().occurrence_id())
            {
                changed |= mate_ids.insert(mate.id());
                changed |= occurrence_ids.insert(mate.endpoint_a().occurrence_id());
                changed |= occurrence_ids.insert(mate.endpoint_b().occurrence_id());
            }
        }
        if !changed {
            break;
        }
    }
    mate_ids
}

fn validate_rebound_mates(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    impact: &SharedChangeImpactProjection,
    producer_transition: bool,
    producer_feature_id: FeatureId,
) -> Result<(), SharedChangePropagationError> {
    for expected in &impact.mate_references {
        let mate = snapshot.assembly_mate(expected.mate_id).ok_or({
            SharedChangePropagationError::Dependency(DependencyBlocker::Mate {
                mate: expected.mate_id,
                problem: MateProblem::Missing,
            })
        })?;
        let endpoint = [mate.endpoint_a(), mate.endpoint_b()]
            .into_iter()
            .find(|endpoint| endpoint.occurrence_id() == expected.occurrence_id)
            .ok_or({
                SharedChangePropagationError::Dependency(DependencyBlocker::Mate {
                    mate: expected.mate_id,
                    problem: MateProblem::OccurrenceLost(expected.occurrence_id),
                })
            })?;
        if endpoint.health() != AssemblyReferenceHealth::Resolved {
            return Err(SharedChangePropagationError::Dependency(
                DependencyBlocker::Mate {
                    mate: expected.mate_id,
                    problem: MateProblem::NotRebound,
                },
            ));
        }
        match exact_results.resolve_reference(snapshot, endpoint.reference()) {
            ExactReferenceResolution::Resolved { reference }
                if reference.lineage_digest == expected.lineage_digest => {}
            ExactReferenceResolution::Resolved { reference }
                if producer_transition && reference.producer_feature_id == producer_feature_id => {}
            ExactReferenceResolution::Resolved { .. } => {
                return Err(SharedChangePropagationError::Dependency(
                    DependencyBlocker::Mate {
                        mate: expected.mate_id,
                        problem: MateProblem::LineageChanged,
                    },
                ));
            }
            resolution => {
                return Err(SharedChangePropagationError::Dependency(
                    DependencyBlocker::Mate {
                        mate: expected.mate_id,
                        problem: MateProblem::NotUniquelyCurrent(resolution),
                    },
                ));
            }
        }
    }
    Ok(())
}

fn refresh_drawings(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    impact: &SharedChangeImpactProjection,
) -> Result<Vec<OrthographicDrawing>, SharedChangePropagationError> {
    let mut drawings = Vec::<OrthographicDrawing>::new();
    for affected in &impact.drawing_views {
        if drawings
            .last()
            .is_some_and(|drawing| drawing.sheet_id == affected.sheet_id)
        {
            continue;
        }
        let sheet = snapshot.drawing_sheet(affected.sheet_id).ok_or({
            SharedChangePropagationError::Dependency(DependencyBlocker::Sheet {
                sheet: affected.sheet_id,
                problem: SheetProblem::Missing,
            })
        })?;
        let drawing =
            project_orthographic_drawing(snapshot, exact_results, sheet).map_err(|error| {
                SharedChangePropagationError::Dependency(DependencyBlocker::Sheet {
                    sheet: affected.sheet_id,
                    problem: SheetProblem::Projection(error),
                })
            })?;
        if !drawing.is_current(snapshot) {
            return Err(SharedChangePropagationError::Dependency(
                DependencyBlocker::Sheet {
                    sheet: affected.sheet_id,
                    problem: SheetProblem::NotRefreshed,
                },
            ));
        }
        drawings.push(drawing);
    }
    Ok(drawings)
}

fn refresh_export_eligibility(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    impact: &SharedChangeImpactProjection,
    body_id: BodyId,
) -> Result<Vec<SharedChangeExportImpact>, SharedChangePropagationError> {
    let scene = snapshot.scene_query();
    let mut refreshed = Vec::with_capacity(impact.exports.len());
    for affected in &impact.exports {
        let mut bodies = Vec::with_capacity(affected.occurrence_paths.len());
        for path in &affected.occurrence_paths {
            let occurrence = scene
                .iter()
                .find(|occurrence| occurrence.instance_path == *path && occurrence.visible)
                .ok_or_else(|| {
                    SharedChangePropagationError::Dependency(DependencyBlocker::Export {
                        path: path.clone(),
                        problem: ExportProblem::NotVisible,
                    })
                })?;
            let package = exact_results
                .get_body(snapshot, occurrence.definition_id, body_id)
                .map_err(|error| {
                    SharedChangePropagationError::Dependency(DependencyBlocker::Exact(error))
                })?
                .ok_or_else(|| {
                    SharedChangePropagationError::Dependency(DependencyBlocker::Export {
                        path: path.clone(),
                        problem: ExportProblem::NoExactBody,
                    })
                })?;
            producer_exact_graph(
                snapshot,
                occurrence.definition_id,
                package.producer_feature_id(),
            )
            .map_err(|error| {
                SharedChangePropagationError::Dependency(DependencyBlocker::Exact(error))
            })?;
            bodies.push((package.as_ref(), occurrence.transform));
        }
        if affected.format == SharedChangeExportFormat::Stl {
            exact_model_stl_export(snapshot, &bodies).map_err(|error| {
                SharedChangePropagationError::Dependency(DependencyBlocker::Exact(error))
            })?;
        }
        refreshed.push(SharedChangeExportImpact {
            format: affected.format,
            occurrence_paths: affected.occurrence_paths.clone(),
            eligibility: SharedChangeExportEligibility::CurrentExact,
        });
    }
    Ok(refreshed)
}

/// The drawing sheets that show the root occurrence `occurrence_id`.
fn selected_sheets(
    snapshot: &Snapshot,
    occurrence_id: OccurrenceId,
) -> impl Iterator<Item = &DrawingSheet> {
    snapshot
        .drawing_sheets()
        .filter(move |sheet| sheet.source().references_root_occurrence(occurrence_id))
}

/// The three orthographic views every drawing sheet projects.
fn sheet_views(sheet_id: DrawingSheetId) -> [SharedChangeDrawingViewImpact; 3] {
    [
        OrthographicViewKind::Front,
        OrthographicViewKind::Top,
        OrthographicViewKind::Right,
    ]
    .map(|view| SharedChangeDrawingViewImpact { sheet_id, view })
}

/// Why a mate endpoint reference is not one current exact subshape.
enum EndpointRefusal {
    Ambiguous,
    Lost,
    Blocked(MateProblem),
}

/// Proves a mate endpoint is healthy and resolves to one current exact
/// subshape in `exact_results`.
fn current_mate_endpoint(
    exact_results: &ExactResultRegistry,
    snapshot: &Snapshot,
    endpoint: &AssemblyMateEndpoint,
) -> Result<(), EndpointRefusal> {
    match endpoint.health() {
        AssemblyReferenceHealth::Resolved => {}
        AssemblyReferenceHealth::Ambiguous { .. } => return Err(EndpointRefusal::Ambiguous),
        AssemblyReferenceHealth::Lost => return Err(EndpointRefusal::Lost),
        AssemblyReferenceHealth::Broken => {
            return Err(EndpointRefusal::Blocked(MateProblem::BrokenReference));
        }
    }
    match exact_results.resolve_reference(snapshot, endpoint.reference()) {
        ExactReferenceResolution::Resolved { .. } => Ok(()),
        ExactReferenceResolution::Ambiguous { .. } => Err(EndpointRefusal::Ambiguous),
        ExactReferenceResolution::Lost => Err(EndpointRefusal::Lost),
        ExactReferenceResolution::Quarantined { reason } => {
            Err(EndpointRefusal::Blocked(MateProblem::Quarantined(reason)))
        }
    }
}

fn validate_stable_lineage(
    previous: &ExactBodyPackage,
    candidate: &ExactBodyPackage,
    producer_transition: bool,
) -> Result<(), SharedChangePropagationError> {
    for previous_reference in previous.references() {
        if let Some(candidate_reference) = candidate.references().iter().find(|reference| {
            reference.semantic_role == previous_reference.semantic_role
                && reference.source_element_id == previous_reference.source_element_id
        }) && candidate_reference.lineage_digest != previous_reference.lineage_digest
            && !producer_transition
        {
            return Err(SharedChangePropagationError::ExactPublication(
                "stable subshape lineage changed during shared-definition evaluation".to_owned(),
            ));
        }
    }
    Ok(())
}

pub fn project_occurrence_fork_impact(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    request: OccurrenceForkChangeRequest,
    principal: ProposalPrincipal,
) -> Result<OccurrenceForkImpactProjection, OccurrenceForkImpactError> {
    let source = document.current();
    if source.revision_id() != request.source_revision
        || source.canonical_digest() != request.source_digest
    {
        return Err(OccurrenceForkImpactError::Stale);
    }
    source
        .feature_dependency_graph()
        .map_err(OccurrenceForkImpactError::DependencyGraph)?;

    let selected = source.occurrence(request.selected_occurrence_id).ok_or(
        OccurrenceForkImpactError::OccurrenceNotFound(request.selected_occurrence_id),
    )?;
    let source_definition_id = selected.definition_id();
    let selected_scene = source
        .scene_query()
        .into_iter()
        .find(|occurrence| {
            occurrence.occurrence_id == request.selected_occurrence_id
                && occurrence.instance_path.is_root()
        })
        .ok_or(OccurrenceForkImpactError::OccurrenceNotFound(
            request.selected_occurrence_id,
        ))?;
    if !selected_scene.visible {
        return Err(OccurrenceForkImpactError::Hidden(
            request.selected_occurrence_id,
        ));
    }

    let mut source_occurrences = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.instance_path.is_root() && occurrence.definition_id == source_definition_id
        })
        .collect::<Vec<_>>();
    source_occurrences.sort_by(|left, right| left.instance_path.cmp(&right.instance_path));
    if source_occurrences.len() < 2 {
        return Err(OccurrenceForkImpactError::DefinitionNotReused(
            source_definition_id,
        ));
    }
    let unchanged_sibling_occurrences = source_occurrences
        .iter()
        .filter(|occurrence| occurrence.occurrence_id != request.selected_occurrence_id)
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path.clone(),
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();

    let requested_definition_id = match &request.change {
        SharedDefinitionChange::ExactParameterEdit(change) => change.definition_id,
        SharedDefinitionChange::BodyHistoryMutation(change) => change.definition_id,
    };
    if requested_definition_id != source_definition_id {
        return Err(OccurrenceForkImpactError::CrossDefinition(
            source_definition_id,
            requested_definition_id,
        ));
    }
    let (body_id, affected_source_feature_ids, validated_commands) = match request.change {
        SharedDefinitionChange::ExactParameterEdit(change) => {
            let body_id = change.body_id;
            let preview =
                prepare_body_parameter_edit(document, change, principal).map_err(|error| {
                    OccurrenceForkImpactError::Unsupported(DependencyBlocker::ParameterEdit(error))
                })?;
            (
                body_id,
                preview.affected_feature_ids,
                preview.proposal.batch().commands().to_vec(),
            )
        }
        SharedDefinitionChange::BodyHistoryMutation(change) => {
            let body_id = change.body_id;
            let preview =
                prepare_body_history_mutation(document, change, principal).map_err(|error| {
                    OccurrenceForkImpactError::Unsupported(DependencyBlocker::HistoryMutation(
                        error,
                    ))
                })?;
            (
                body_id,
                preview.affected_feature_ids,
                preview.proposal.batch().commands().to_vec(),
            )
        }
    };

    let source_definition = source.definition(source_definition_id).ok_or({
        OccurrenceForkImpactError::Unsupported(DependencyBlocker::DefinitionMissing(
            source_definition_id,
        ))
    })?;
    let last_valid = current_body_result(exact_results, &source, source_definition_id, body_id)
        .map_err(map_occurrence_fork_impact_error)?;

    let fork_definition_id = DefinitionId(next_fork_id(
        source.definitions().map(|definition| definition.id().0),
    )?);
    let mut next_feature_id = next_fork_id(source.features().map(|feature| feature.id().0))?;
    let mut feature_id_map = Vec::with_capacity(source_definition.feature_ids().len());
    for source_feature_id in source_definition.feature_ids() {
        let fork_feature_id = FeatureId(next_feature_id);
        feature_id_map.push((*source_feature_id, fork_feature_id));
        next_feature_id = next_feature_id.checked_add(1).ok_or({
            OccurrenceForkImpactError::Unsupported(DependencyBlocker::IdentitySpaceExhausted)
        })?;
    }
    let feature_lineage = feature_id_map
        .iter()
        .map(
            |(source_feature_id, fork_feature_id)| OccurrenceForkFeatureLineage {
                source_feature_id: *source_feature_id,
                fork_feature_id: *fork_feature_id,
            },
        )
        .collect::<Vec<_>>();
    let mapped_feature = |source_id: FeatureId| {
        feature_id_map
            .iter()
            .find_map(|(source, fork)| (*source == source_id).then_some(*fork))
            .ok_or({
                OccurrenceForkImpactError::Unsupported(DependencyBlocker::FeatureOutsideDefinition(
                    source_id,
                ))
            })
    };

    let mut mate_references = Vec::new();
    let mut mate_rebind_commands = Vec::new();
    for mate in source.assembly_mates() {
        let mut rebound_endpoints = Vec::with_capacity(2);
        let mut changed = false;
        for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
            if endpoint.occurrence_id() != request.selected_occurrence_id {
                rebound_endpoints.push(endpoint.clone());
                continue;
            }
            if endpoint.reference().definition_id != source_definition_id {
                return Err(OccurrenceForkImpactError::CrossDefinition(
                    source_definition_id,
                    endpoint.reference().definition_id,
                ));
            }
            current_mate_endpoint(exact_results, &source, endpoint).map_err(
                |refusal| match refusal {
                    EndpointRefusal::Ambiguous => OccurrenceForkImpactError::Ambiguous(body_id),
                    EndpointRefusal::Lost => OccurrenceForkImpactError::Lost(mate.id()),
                    EndpointRefusal::Blocked(problem) => {
                        OccurrenceForkImpactError::Unsupported(DependencyBlocker::Mate {
                            mate: mate.id(),
                            problem,
                        })
                    }
                },
            )?;
            let fork_profile_feature_id = mapped_feature(endpoint.reference().profile_feature_id)?;
            let fork_producer_feature_id =
                mapped_feature(endpoint.reference().producer_feature_id)?;
            let fork_lineage_digest = canonical_reference_lineage_digest(
                source.document_id(),
                fork_producer_feature_id,
                &endpoint.reference().semantic_role,
                &endpoint.reference().source_element_id,
                &endpoint.reference().expected_type,
            );
            mate_references.push(OccurrenceForkMateReferenceImpact {
                mate_id: mate.id(),
                occurrence_id: request.selected_occurrence_id,
                source_definition_id,
                source_producer_feature_id: endpoint.reference().producer_feature_id,
                source_lineage_digest: endpoint.reference().lineage_digest.clone(),
                fork_definition_id,
                fork_producer_feature_id,
                fork_lineage_digest: fork_lineage_digest.clone(),
            });
            let mut fork_reference = endpoint.reference().clone();
            fork_reference.definition_id = fork_definition_id;
            fork_reference.profile_feature_id = fork_profile_feature_id;
            fork_reference.producer_feature_id = fork_producer_feature_id;
            fork_reference.lineage_digest = fork_lineage_digest;
            rebound_endpoints.push(AssemblyMateEndpoint::resolved(
                request.selected_occurrence_id,
                fork_reference,
            ));
            changed = true;
        }
        if changed {
            let [endpoint_a, endpoint_b]: [AssemblyMateEndpoint; 2] = rebound_endpoints
                .try_into()
                .expect("every mate has exactly two endpoints");
            mate_rebind_commands.push(CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
                mate.id(),
                endpoint_a,
                endpoint_b,
                mate.kind(),
            )));
        }
    }
    mate_references.sort_by(|left, right| {
        (
            left.mate_id,
            left.occurrence_id,
            &left.source_lineage_digest,
        )
            .cmp(&(
                right.mate_id,
                right.occurrence_id,
                &right.source_lineage_digest,
            ))
    });

    let mut commands = vec![CanonicalCommand::CloneDefinitionAndRepoint(
        CloneDefinitionPlan::new(
            request.selected_occurrence_id,
            source_definition_id,
            fork_definition_id,
            request.new_definition_name,
            feature_id_map.clone(),
        ),
    )];
    commands.extend(mate_rebind_commands);
    for command in validated_commands {
        commands.push(match command {
            CanonicalCommand::SetFeatureDimension { id, dimension } => {
                CanonicalCommand::SetFeatureDimension {
                    id: mapped_feature(id)?,
                    dimension,
                }
            }
            CanonicalCommand::SetFeatureParameter {
                mut target,
                dimension,
            } => {
                target.feature_id = mapped_feature(target.feature_id)?;
                CanonicalCommand::SetFeatureParameter { target, dimension }
            }
            CanonicalCommand::SetSketchConstraintDimension {
                id,
                constraint_id,
                dimension,
            } => CanonicalCommand::SetSketchConstraintDimension {
                id: mapped_feature(id)?,
                constraint_id,
                dimension,
            },
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                suppressed_feature_ids,
            } if definition_id == source_definition_id => {
                CanonicalCommand::SetBodyFeatureSuppression {
                    definition_id: fork_definition_id,
                    body_id,
                    suppressed_feature_ids: suppressed_feature_ids
                        .into_iter()
                        .map(mapped_feature)
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            _ => {
                return Err(OccurrenceForkImpactError::Unsupported(
                    DependencyBlocker::ChangeKindNotForkable,
                ));
            }
        });
    }

    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(commands),
            ProposalContext {
                principal,
                goal: ProposalGoal::CanonicalPreview,
                assumptions: vec![
                    ProposalAssumption::TargetExists(AuthoritativeDependency::Occurrence(
                        request.selected_occurrence_id,
                    )),
                    ProposalAssumption::TargetExists(AuthoritativeDependency::Definition(
                        source_definition_id,
                    )),
                    ProposalAssumption::TargetExists(AuthoritativeDependency::DefinitionUsers(
                        source_definition_id,
                    )),
                    ProposalAssumption::TargetMissing(AuthoritativeDependency::Definition(
                        fork_definition_id,
                    )),
                ],
                risk: ProposalRisk::Standard,
                confirmation: ProposalConfirmation::ReviewRequired,
                requested_budget: ProposalBudget::HOST_MAX,
            },
        )
        .map_err(|error| {
            OccurrenceForkImpactError::Unsupported(DependencyBlocker::Proposal(error))
        })?;
    let candidate = document.preview_batch(proposal.batch()).map_err(|error| {
        OccurrenceForkImpactError::Unsupported(DependencyBlocker::Staging(error))
    })?;
    if candidate.definition(source_definition_id) != Some(source_definition) {
        return Err(OccurrenceForkImpactError::Unsupported(
            DependencyBlocker::CandidateChanged(CandidateDrift::Definition(source_definition_id)),
        ));
    }

    let exact_graph =
        body_exact_graph(&candidate, fork_definition_id, body_id).map_err(|error| match error {
            ExactProductError::ConflictingBodyTerminals { .. }
            | ExactProductError::ConflictingBodyPublication { .. } => {
                OccurrenceForkImpactError::Ambiguous(body_id)
            }
            error => OccurrenceForkImpactError::Unsupported(DependencyBlocker::Exact(error)),
        })?;
    let exact_jobs = vec![SharedChangeExactJob {
        definition_id: fork_definition_id,
        body_id,
        producer_feature_id: FeatureId(exact_graph.producer_feature_id),
        canonical_input_digest: exact_graph.canonical_input_digest.clone(),
        last_valid_result_fingerprint: last_valid.result_key().result_fingerprint.clone(),
    }];

    let body_lineage = source_definition
        .bodies()
        .map(|body| OccurrenceForkBodyLineage {
            source_definition_id,
            source_body_id: body.id(),
            fork_definition_id,
            fork_body_id: body.id(),
        })
        .collect::<Vec<_>>();
    let affected_fork_feature_ids = affected_source_feature_ids
        .iter()
        .copied()
        .map(mapped_feature)
        .collect::<Result<Vec<_>, _>>()?;

    let source_feature = |fork_id: FeatureId| {
        feature_id_map
            .iter()
            .find_map(|(source, fork)| (*fork == fork_id).then_some(*source))
            .ok_or({
                OccurrenceForkImpactError::Unsupported(
                    DependencyBlocker::ForkFeatureWithoutLineage(fork_id),
                )
            })
    };
    let fork_producer_feature_id = FeatureId(exact_graph.producer_feature_id);
    let source_producer_feature_id = source_feature(fork_producer_feature_id)?;
    // A fork copies the source definition, so it produces the same faces as the
    // source body's last valid exact result.
    let mut subshape_lineage = last_valid
        .references()
        .iter()
        .map(|reference| {
            let source_profile_feature_id = reference.profile_feature_id;
            let fork_profile_feature_id = mapped_feature(source_profile_feature_id)?;
            Ok(OccurrenceForkSubshapeLineage {
                source_definition_id,
                source_profile_feature_id,
                source_producer_feature_id,
                source_lineage_digest: canonical_reference_lineage_digest(
                    source.document_id(),
                    source_producer_feature_id,
                    &reference.semantic_role,
                    &reference.source_element_id,
                    &reference.expected_type,
                ),
                fork_definition_id,
                fork_profile_feature_id,
                fork_producer_feature_id,
                fork_lineage_digest: canonical_reference_lineage_digest(
                    source.document_id(),
                    fork_producer_feature_id,
                    &reference.semantic_role,
                    &reference.source_element_id,
                    &reference.expected_type,
                ),
                semantic_role: reference.semantic_role.clone(),
                source_element_id: reference.source_element_id.clone(),
                expected_type: reference.expected_type.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    subshape_lineage.sort_by(|left, right| {
        (
            left.source_profile_feature_id,
            left.source_producer_feature_id,
            &left.semantic_role,
            &left.source_element_id,
            &left.expected_type,
        )
            .cmp(&(
                right.source_profile_feature_id,
                right.source_producer_feature_id,
                &right.semantic_role,
                &right.source_element_id,
                &right.expected_type,
            ))
    });

    let mut drawing_views = selected_sheets(&source, request.selected_occurrence_id)
        .flat_map(|sheet| sheet_views(sheet.id()))
        .collect::<Vec<_>>();
    drawing_views.sort_unstable();

    let exports = [
        SharedChangeExportFormat::Step,
        SharedChangeExportFormat::Stl,
    ]
    .into_iter()
    .map(|format| SharedChangeExportImpact {
        format,
        occurrence_paths: vec![selected_scene.instance_path.clone()],
        eligibility: SharedChangeExportEligibility::PendingExactRecompute,
    })
    .collect();

    Ok(OccurrenceForkImpactProjection {
        source_revision: source.revision_id(),
        source_digest: source.canonical_digest(),
        candidate_digest: candidate.canonical_digest(),
        selected_occurrence_id: request.selected_occurrence_id,
        selected_instance_path: selected_scene.instance_path,
        source_definition_id,
        fork_definition_id,
        body_lineage,
        feature_lineage,
        subshape_lineage,
        affected_fork_body_ids: vec![body_id],
        affected_fork_feature_ids,
        unchanged_source_body_ids: source_definition.bodies().map(|body| body.id()).collect(),
        unchanged_sibling_occurrences,
        unchanged_definition_ids: source
            .definitions()
            .map(|definition| definition.id())
            .collect(),
        exact_jobs,
        mate_references,
        collection_dependencies: preserved_collection_dependencies(
            &source,
            request.selected_occurrence_id,
        ),
        drawing_views,
        exports,
        proposal,
    })
}

fn next_fork_id(ids: impl Iterator<Item = u64>) -> Result<u64, OccurrenceForkImpactError> {
    ids.max().unwrap_or(0).checked_add(1).ok_or({
        OccurrenceForkImpactError::Unsupported(DependencyBlocker::IdentitySpaceExhausted)
    })
}

fn map_occurrence_fork_impact_error(error: SharedChangeImpactError) -> OccurrenceForkImpactError {
    match error {
        SharedChangeImpactError::Stale => OccurrenceForkImpactError::Stale,
        SharedChangeImpactError::DefinitionNotReused(id) => {
            OccurrenceForkImpactError::DefinitionNotReused(id)
        }
        SharedChangeImpactError::Failed(body_id) => OccurrenceForkImpactError::Failed(body_id),
        SharedChangeImpactError::Ambiguous(body_id) => {
            OccurrenceForkImpactError::Ambiguous(body_id)
        }
        SharedChangeImpactError::Lost(mate_id) => OccurrenceForkImpactError::Lost(mate_id),
        SharedChangeImpactError::DependencyGraph(error) => {
            OccurrenceForkImpactError::DependencyGraph(error)
        }
        SharedChangeImpactError::Unsupported(reason) => {
            OccurrenceForkImpactError::Unsupported(reason)
        }
    }
}

pub fn project_shared_change_impact(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    request: SharedDefinitionChangeRequest,
    principal: ProposalPrincipal,
) -> Result<SharedChangeImpactProjection, SharedChangeImpactError> {
    let source = document.current();
    if source.revision_id() != request.source_revision
        || source.canonical_digest() != request.source_digest
    {
        return Err(SharedChangeImpactError::Stale);
    }
    source
        .feature_dependency_graph()
        .map_err(SharedChangeImpactError::DependencyGraph)?;

    let (
        definition_id,
        body_id,
        affected_body_ids,
        affected_feature_ids,
        unchanged_body_ids,
        proposal,
    ) = match request.change {
        SharedDefinitionChange::ExactParameterEdit(change) => {
            let definition_id = change.definition_id;
            let body_id = change.body_id;
            let preview =
                prepare_dependency_staging_body_parameter_edit(document, change, principal)
                    .map_err(|error| {
                        SharedChangeImpactError::Unsupported(DependencyBlocker::ParameterEdit(
                            error,
                        ))
                    })?;
            (
                definition_id,
                body_id,
                preview.affected_body_ids,
                preview.affected_feature_ids,
                preview.unchanged_body_ids,
                preview.proposal,
            )
        }
        SharedDefinitionChange::BodyHistoryMutation(change) => {
            let definition_id = change.definition_id;
            let body_id = change.body_id;
            let preview =
                prepare_body_history_mutation(document, change, principal).map_err(|error| {
                    SharedChangeImpactError::Unsupported(DependencyBlocker::HistoryMutation(error))
                })?;
            (
                definition_id,
                body_id,
                vec![body_id],
                preview.affected_feature_ids,
                preview.unchanged_body_ids,
                preview.proposal,
            )
        }
    };

    let mut occurrences = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.definition_id == definition_id)
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path,
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();
    occurrences.sort_by(|left, right| left.instance_path.cmp(&right.instance_path));
    if occurrences.len() < 2 {
        return Err(SharedChangeImpactError::DefinitionNotReused(definition_id));
    }

    let candidate = document
        .preview_dependency_staging_batch(proposal.batch())
        .map_err(|error| SharedChangeImpactError::Unsupported(DependencyBlocker::Staging(error)))?;
    let terminal_graphs = terminal_body_exact_graphs(&candidate, definition_id)
        .map_err(|error| map_exact_request_error(error, body_id))?;
    let exact_jobs = terminal_graphs
        .into_iter()
        .filter(|(terminal_body_id, _)| affected_body_ids.contains(terminal_body_id))
        .map(|(terminal_body_id, exact_graph)| {
            let last_valid =
                current_body_result(exact_results, &source, definition_id, terminal_body_id)?;
            Ok(SharedChangeExactJob {
                definition_id,
                body_id: terminal_body_id,
                producer_feature_id: FeatureId(exact_graph.producer_feature_id),
                canonical_input_digest: exact_graph.canonical_input_digest,
                last_valid_result_fingerprint: last_valid.result_key().result_fingerprint,
            })
        })
        .collect::<Result<Vec<_>, SharedChangeImpactError>>()?;
    if exact_jobs.is_empty() {
        return Err(SharedChangeImpactError::Failed(body_id));
    }

    let affected_roots = occurrences
        .iter()
        .map(|occurrence| occurrence.instance_path.root_occurrence())
        .collect::<Vec<_>>();
    let mut mate_references = Vec::new();
    for mate in source.assembly_mates() {
        for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
            if !affected_roots.contains(&endpoint.occurrence_id())
                || endpoint.reference().definition_id != definition_id
            {
                continue;
            }
            current_mate_endpoint(exact_results, &source, endpoint).map_err(
                |refusal| match refusal {
                    EndpointRefusal::Ambiguous => SharedChangeImpactError::Ambiguous(body_id),
                    EndpointRefusal::Lost => SharedChangeImpactError::Lost(mate.id()),
                    EndpointRefusal::Blocked(problem) => {
                        SharedChangeImpactError::Unsupported(DependencyBlocker::Mate {
                            mate: mate.id(),
                            problem,
                        })
                    }
                },
            )?;
            mate_references.push(SharedChangeMateReferenceImpact {
                mate_id: mate.id(),
                occurrence_id: endpoint.occurrence_id(),
                definition_id,
                producer_feature_id: endpoint.reference().producer_feature_id,
                lineage_digest: endpoint.reference().lineage_digest.clone(),
            });
        }
    }
    mate_references.sort_by(|left, right| {
        (left.mate_id, left.occurrence_id, &left.lineage_digest).cmp(&(
            right.mate_id,
            right.occurrence_id,
            &right.lineage_digest,
        ))
    });

    let mut drawing_views = Vec::new();
    for sheet in source.drawing_sheets() {
        let affected = match sheet.source() {
            DrawingSource::Definition(id) => *id == definition_id,
            DrawingSource::RigidAssembly { occurrence_ids } => occurrence_ids
                .iter()
                .any(|occurrence_id| affected_roots.contains(occurrence_id)),
            DrawingSource::RigidAssemblyInstances { instance_paths } => instance_paths
                .iter()
                .any(|path| affected_roots.contains(&path.root_occurrence())),
        };
        if affected {
            drawing_views.extend(sheet_views(sheet.id()));
        }
    }
    drawing_views.sort_unstable();

    let visible_paths = occurrences
        .iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.instance_path.clone())
        .collect::<Vec<_>>();
    let exports = if visible_paths.is_empty() {
        Vec::new()
    } else {
        [
            SharedChangeExportFormat::Step,
            SharedChangeExportFormat::Stl,
        ]
        .into_iter()
        .map(|format| SharedChangeExportImpact {
            format,
            occurrence_paths: visible_paths.clone(),
            eligibility: SharedChangeExportEligibility::PendingExactRecompute,
        })
        .collect()
    };

    Ok(SharedChangeImpactProjection {
        source_revision: source.revision_id(),
        source_digest: source.canonical_digest(),
        candidate_digest: candidate.canonical_digest(),
        definition_id,
        affected_body_ids,
        affected_feature_ids,
        unchanged_body_ids,
        unchanged_definition_ids: source
            .definitions()
            .filter_map(|definition| (definition.id() != definition_id).then_some(definition.id()))
            .collect(),
        occurrences,
        exact_jobs,
        mate_references,
        drawing_views,
        exports,
        proposal,
    })
}

fn current_body_result<'a>(
    exact_results: &'a ExactResultRegistry,
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    body_id: BodyId,
) -> Result<&'a Arc<ExactBodyPackage>, SharedChangeImpactError> {
    match exact_results.get_body(snapshot, definition_id, body_id) {
        Ok(Some(package)) => Ok(package),
        Ok(None) => {
            let has_previous = exact_results.values().any(|package| {
                package.definition_id() == definition_id
                    && snapshot
                        .definition(definition_id)
                        .and_then(|definition| {
                            definition.feature_body_ownership(package.producer_feature_id())
                        })
                        .and_then(|ownership| ownership.output_body_id())
                        == Some(body_id)
            });
            if has_previous {
                Err(SharedChangeImpactError::Stale)
            } else {
                Err(SharedChangeImpactError::Failed(body_id))
            }
        }
        Err(ExactProductError::ConflictingBodyPublication { .. }) => {
            Err(SharedChangeImpactError::Ambiguous(body_id))
        }
        Err(error) => Err(SharedChangeImpactError::Unsupported(
            DependencyBlocker::Exact(error),
        )),
    }
}

fn map_exact_request_error(error: ExactProductError, body_id: BodyId) -> SharedChangeImpactError {
    match error {
        ExactProductError::ConflictingBodyTerminals { .. }
        | ExactProductError::ConflictingBodyPublication { .. } => {
            SharedChangeImpactError::Ambiguous(body_id)
        }
        error => SharedChangeImpactError::Unsupported(DependencyBlocker::Exact(error)),
    }
}
