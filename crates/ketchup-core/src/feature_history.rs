use crate::document::{
    AuthoritativeDependency, BodyId, BooleanOperation, CanonicalCommand, CommandBatch,
    DefinitionId, Dimension, DocumentStore, FeatureDependencyGraph, FeatureId, FeatureKind,
    Proposal, ProposalAssumption, ProposalConfirmation, ProposalContext, ProposalGoal,
    ProposalPrepareError, ProposalPrincipal, ProposalRisk, Snapshot,
};
use crate::exact_product::{
    BodySubshapeRef, ExactFeatureChainRequest, ExactReferenceQuarantineReason,
    ExactReferenceResolution, ExactResultRegistry,
};
use crate::sketch::{SketchConstraintId, SketchConstraintKind, WorkplaneSupport};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureHistoryState {
    Active,
    RollbackSuppressed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureHistoryEntry {
    pub feature_id: FeatureId,
    pub name: String,
    pub topological_index: usize,
    pub dependencies: Vec<FeatureId>,
    pub dependents: Vec<FeatureId>,
    pub input_body_ids: Vec<BodyId>,
    pub output_body_id: Option<BodyId>,
    pub state: FeatureHistoryState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyFeatureHistory {
    pub body_id: BodyId,
    pub name: String,
    pub visible: bool,
    pub active: bool,
    pub consumed_by: Option<FeatureId>,
    pub features: Vec<FeatureHistoryEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RollbackPreviewRequest {
    pub body_id: BodyId,
    pub first_suppressed_feature_id: FeatureId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackPreview {
    pub body_id: BodyId,
    pub first_suppressed_feature_id: FeatureId,
    pub suppressed_feature_ids: Vec<FeatureId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedFeatureProvenance {
    pub feature_id: FeatureId,
    pub definition_id: DefinitionId,
    pub body_ids: Vec<BodyId>,
    pub dependencies: Vec<FeatureId>,
    pub input_body_ids: Vec<BodyId>,
    pub output_body_id: Option<BodyId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedSubshapeProvenance {
    pub body_id: BodyId,
    pub profile_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub semantic_role: String,
    pub source_element_id: String,
    pub lineage_digest: String,
    pub result_fingerprint: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FeatureHistoryQuery {
    pub selected_feature_id: Option<FeatureId>,
    pub selected_subshape: Option<BodySubshapeRef>,
    pub rollback_preview: Option<RollbackPreviewRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureHistoryProjection {
    pub revision_id: u64,
    pub canonical_digest: String,
    pub definition_id: DefinitionId,
    pub bodies: Vec<BodyFeatureHistory>,
    pub selected_feature: Option<SelectedFeatureProvenance>,
    pub selected_subshape: Option<SelectedSubshapeProvenance>,
    pub rollback_preview: Option<RollbackPreview>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ExactParameterEditTarget {
    FeatureDimension(FeatureId),
    SketchConstraintDimension {
        sketch_id: FeatureId,
        constraint_id: SketchConstraintId,
    },
}

impl ExactParameterEditTarget {
    const fn feature_id(self) -> FeatureId {
        match self {
            Self::FeatureDimension(id) => id,
            Self::SketchConstraintDimension { sketch_id, .. } => sketch_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactParameterEdit {
    pub target: ExactParameterEditTarget,
    pub dimension: Dimension,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyParameterEditRequest {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub edits: Vec<ExactParameterEdit>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyProfileTranslationRequest {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub profile_id: FeatureId,
    pub delta_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyParameterEditPreview {
    pub source_revision: u64,
    pub source_digest: String,
    pub body_id: BodyId,
    pub affected_feature_ids: Vec<FeatureId>,
    pub unchanged_body_ids: Vec<BodyId>,
    pub proposal: Proposal,
}

#[derive(Debug, PartialEq)]
pub enum BodyParameterEditError {
    Empty,
    Duplicate(ExactParameterEditTarget),
    DefinitionNotFound(DefinitionId),
    BodyNotFound(DefinitionId, BodyId),
    FeatureNotFound(FeatureId),
    FeatureOutsideDefinition(FeatureId, DefinitionId),
    FeatureOutsideBody(FeatureId, BodyId),
    CrossBodyAffected(FeatureId, BodyId),
    UnsupportedTarget(ExactParameterEditTarget),
    InvalidCutPosition,
    History(FeatureHistoryError),
    Proposal(ProposalPrepareError),
}

impl fmt::Display for BodyParameterEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("body parameter edit is empty"),
            Self::Duplicate(target) => write!(formatter, "duplicate parameter edit: {target:?}"),
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} was not found", id.0),
            Self::BodyNotFound(definition, body) => write!(
                formatter,
                "body {} was not found in definition {}",
                body.0, definition.0
            ),
            Self::FeatureNotFound(id) => write!(formatter, "feature {} was not found", id.0),
            Self::FeatureOutsideDefinition(feature, definition) => write!(
                formatter,
                "feature {} does not belong to definition {}",
                feature.0, definition.0
            ),
            Self::FeatureOutsideBody(feature, body) => write!(
                formatter,
                "feature {} does not belong to body {} history",
                feature.0, body.0
            ),
            Self::CrossBodyAffected(feature, body) => write!(
                formatter,
                "feature {} would affect unrelated body {}",
                feature.0, body.0
            ),
            Self::UnsupportedTarget(target) => {
                write!(formatter, "unsupported exact parameter target: {target:?}")
            }
            Self::InvalidCutPosition => formatter.write_str(
                "moving the cut profile would leave the supported host or invalidate exact geometry",
            ),
            Self::History(error) => error.fmt(formatter),
            Self::Proposal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BodyParameterEditError {}

pub fn prepare_body_parameter_edit(
    document: &DocumentStore,
    request: BodyParameterEditRequest,
    principal: ProposalPrincipal,
) -> Result<BodyParameterEditPreview, BodyParameterEditError> {
    if request.edits.is_empty() {
        return Err(BodyParameterEditError::Empty);
    }
    let snapshot = document.current();
    let definition = snapshot.definition(request.definition_id).ok_or(
        BodyParameterEditError::DefinitionNotFound(request.definition_id),
    )?;
    if definition.body(request.body_id).is_none() {
        return Err(BodyParameterEditError::BodyNotFound(
            request.definition_id,
            request.body_id,
        ));
    }
    let graph = snapshot.feature_dependency_graph().map_err(|error| {
        BodyParameterEditError::Proposal(ProposalPrepareError::Canonical(error))
    })?;
    let body_history = collect_body_history(
        &snapshot,
        request.definition_id,
        request.body_id,
        &graph,
        graph.topological_order(),
    )
    .map_err(BodyParameterEditError::History)?;

    let mut targets = BTreeSet::new();
    let mut roots = BTreeSet::new();
    let mut commands = Vec::with_capacity(request.edits.len());
    let mut assumptions = Vec::with_capacity(request.edits.len());
    for edit in request.edits {
        if !targets.insert(edit.target) {
            return Err(BodyParameterEditError::Duplicate(edit.target));
        }
        let feature_id = edit.target.feature_id();
        let feature = snapshot
            .feature(feature_id)
            .ok_or(BodyParameterEditError::FeatureNotFound(feature_id))?;
        if feature.definition_id() != request.definition_id {
            return Err(BodyParameterEditError::FeatureOutsideDefinition(
                feature_id,
                request.definition_id,
            ));
        }
        if !body_history.contains(&feature_id) {
            return Err(BodyParameterEditError::FeatureOutsideBody(
                feature_id,
                request.body_id,
            ));
        }
        let command = match edit.target {
            ExactParameterEditTarget::FeatureDimension(id)
                if matches!(
                    feature.kind(),
                    FeatureKind::Extrusion { .. }
                        | FeatureKind::Pad(_)
                        | FeatureKind::SketchPocket(_)
                        | FeatureKind::Pocket { .. }
                ) || matches!(
                    feature.kind(),
                    FeatureKind::Workplane(spec)
                        if matches!(&spec.support, WorkplaneSupport::Offset { .. })
                ) =>
            {
                CanonicalCommand::SetFeatureDimension {
                    id,
                    dimension: edit.dimension,
                }
            }
            ExactParameterEditTarget::SketchConstraintDimension {
                sketch_id,
                constraint_id,
            } if matches!(feature.kind(), FeatureKind::Sketch(spec) if spec.constraints.iter().any(
                |constraint| constraint.id == constraint_id
                    && matches!(&constraint.kind, SketchConstraintKind::Distance { .. } | SketchConstraintKind::Radius { .. } | SketchConstraintKind::Angle { .. })
            )) =>
            {
                CanonicalCommand::SetSketchConstraintDimension {
                    id: sketch_id,
                    constraint_id,
                    dimension: edit.dimension,
                }
            }
            target => return Err(BodyParameterEditError::UnsupportedTarget(target)),
        };
        roots.insert(feature_id);
        assumptions.push(ProposalAssumption::TargetExists(
            AuthoritativeDependency::Feature(feature_id),
        ));
        commands.push(command);
    }

    let affected = graph.dependent_closure(roots);
    for feature_id in &affected {
        let feature = snapshot
            .feature(*feature_id)
            .ok_or(BodyParameterEditError::FeatureNotFound(*feature_id))?;
        if feature.definition_id() != request.definition_id {
            return Err(BodyParameterEditError::FeatureOutsideDefinition(
                *feature_id,
                request.definition_id,
            ));
        }
        let ownership = definition
            .feature_body_ownership(*feature_id)
            .ok_or(BodyParameterEditError::FeatureNotFound(*feature_id))?;
        if let Some(output_body_id) = ownership.output_body_id()
            && output_body_id != request.body_id
        {
            return Err(BodyParameterEditError::CrossBodyAffected(
                *feature_id,
                output_body_id,
            ));
        }
    }

    let context = ProposalContext {
        principal,
        goal: ProposalGoal::CanonicalPreview,
        assumptions,
        risk: ProposalRisk::Standard,
        confirmation: ProposalConfirmation::ReviewRequired,
        requested_budget: crate::document::ProposalBudget::HOST_MAX,
    };
    let proposal = document
        .prepare_proposal_with_context(CommandBatch::new(commands), context)
        .map_err(BodyParameterEditError::Proposal)?;
    Ok(BodyParameterEditPreview {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        body_id: request.body_id,
        affected_feature_ids: affected.into_iter().collect(),
        unchanged_body_ids: definition
            .bodies()
            .filter_map(|body| (body.id() != request.body_id).then_some(body.id()))
            .collect(),
        proposal,
    })
}

pub fn prepare_body_profile_translation(
    document: &DocumentStore,
    request: BodyProfileTranslationRequest,
    principal: ProposalPrincipal,
) -> Result<BodyParameterEditPreview, BodyParameterEditError> {
    if !request.delta_mm.iter().all(|value| value.is_finite())
        || (request.delta_mm[0] == 0.0 && request.delta_mm[1] == 0.0)
    {
        return Err(BodyParameterEditError::Empty);
    }
    let snapshot = document.current();
    let definition = snapshot.definition(request.definition_id).ok_or(
        BodyParameterEditError::DefinitionNotFound(request.definition_id),
    )?;
    if definition.body(request.body_id).is_none() {
        return Err(BodyParameterEditError::BodyNotFound(
            request.definition_id,
            request.body_id,
        ));
    }
    let feature = snapshot
        .feature(request.profile_id)
        .ok_or(BodyParameterEditError::FeatureNotFound(request.profile_id))?;
    if feature.definition_id() != request.definition_id {
        return Err(BodyParameterEditError::FeatureOutsideDefinition(
            request.profile_id,
            request.definition_id,
        ));
    }
    if !matches!(
        feature.kind(),
        FeatureKind::Sketch(_)
            | FeatureKind::Profile { .. }
            | FeatureKind::SegmentProfile { .. }
            | FeatureKind::SplineProfile { .. }
    ) {
        return Err(BodyParameterEditError::UnsupportedTarget(
            ExactParameterEditTarget::FeatureDimension(request.profile_id),
        ));
    }
    let graph = snapshot.feature_dependency_graph().map_err(|error| {
        BodyParameterEditError::Proposal(ProposalPrepareError::Canonical(error))
    })?;
    let body_history = collect_body_history(
        &snapshot,
        request.definition_id,
        request.body_id,
        &graph,
        graph.topological_order(),
    )
    .map_err(BodyParameterEditError::History)?;
    if !body_history.contains(&request.profile_id) {
        return Err(BodyParameterEditError::FeatureOutsideBody(
            request.profile_id,
            request.body_id,
        ));
    }
    let affected = graph.dependent_closure(BTreeSet::from([request.profile_id]));
    let is_cut_profile = affected.iter().any(|feature_id| {
        snapshot
            .feature(*feature_id)
            .is_some_and(|feature| match feature.kind() {
                FeatureKind::SketchPocket(spec) => spec.sketch == request.profile_id,
                FeatureKind::ThroughCut { profile, .. } | FeatureKind::Pocket { profile, .. } => {
                    *profile == request.profile_id
                }
                FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    tool,
                    ..
                } => snapshot.feature(*tool).is_some_and(|tool| {
                    matches!(
                        tool.kind(),
                        FeatureKind::Extrusion { profile, .. } if *profile == request.profile_id
                    )
                }),
                _ => false,
            })
    });
    if !is_cut_profile {
        return Err(BodyParameterEditError::UnsupportedTarget(
            ExactParameterEditTarget::FeatureDimension(request.profile_id),
        ));
    }
    for feature_id in &affected {
        let feature = snapshot
            .feature(*feature_id)
            .ok_or(BodyParameterEditError::FeatureNotFound(*feature_id))?;
        if feature.definition_id() != request.definition_id {
            return Err(BodyParameterEditError::FeatureOutsideDefinition(
                *feature_id,
                request.definition_id,
            ));
        }
        let ownership = definition
            .feature_body_ownership(*feature_id)
            .ok_or(BodyParameterEditError::FeatureNotFound(*feature_id))?;
        if let Some(output_body_id) = ownership.output_body_id()
            && output_body_id != request.body_id
        {
            return Err(BodyParameterEditError::CrossBodyAffected(
                *feature_id,
                output_body_id,
            ));
        }
    }

    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::TranslateProfile {
                id: request.profile_id,
                delta_mm: request.delta_mm,
            }]),
            ProposalContext {
                principal,
                goal: ProposalGoal::CanonicalPreview,
                assumptions: vec![ProposalAssumption::TargetExists(
                    AuthoritativeDependency::Feature(request.profile_id),
                )],
                risk: ProposalRisk::Standard,
                confirmation: ProposalConfirmation::ReviewRequired,
                requested_budget: crate::document::ProposalBudget::HOST_MAX,
            },
        )
        .map_err(BodyParameterEditError::Proposal)?;
    let candidate = document.preview_batch(proposal.batch()).map_err(|error| {
        BodyParameterEditError::Proposal(ProposalPrepareError::Canonical(error))
    })?;
    ExactFeatureChainRequest::from_snapshot_for_body(
        &candidate,
        request.definition_id,
        request.body_id,
    )
    .map_err(|_| BodyParameterEditError::InvalidCutPosition)?;
    Ok(BodyParameterEditPreview {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        body_id: request.body_id,
        affected_feature_ids: affected.into_iter().collect(),
        unchanged_body_ids: definition
            .bodies()
            .filter_map(|body| (body.id() != request.body_id).then_some(body.id()))
            .collect(),
        proposal,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureHistoryError {
    DefinitionNotFound(DefinitionId),
    BodyNotFound(DefinitionId, BodyId),
    FeatureNotFound(FeatureId),
    FeatureOutsideDefinition(FeatureId, DefinitionId),
    FeatureOutsideBody(FeatureId, BodyId),
    RollbackNotDependencyClosed(FeatureId),
    SelectionConflict(FeatureId, FeatureId),
    SubshapeWrongDocument,
    SubshapeWrongDefinition(DefinitionId, DefinitionId),
    SubshapeProducerHasNoBody(FeatureId),
    SubshapeAmbiguous(usize),
    SubshapeLost,
    SubshapeQuarantined(ExactReferenceQuarantineReason),
    InvalidFeatureGraph,
}

impl fmt::Display for FeatureHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} was not found", id.0),
            Self::BodyNotFound(definition, body) => write!(
                formatter,
                "body {} was not found in definition {}",
                body.0, definition.0
            ),
            Self::FeatureNotFound(id) => write!(formatter, "feature {} was not found", id.0),
            Self::FeatureOutsideDefinition(feature, definition) => write!(
                formatter,
                "feature {} does not belong to definition {}",
                feature.0, definition.0
            ),
            Self::FeatureOutsideBody(feature, body) => write!(
                formatter,
                "feature {} does not belong to body {} history",
                feature.0, body.0
            ),
            Self::RollbackNotDependencyClosed(feature) => write!(
                formatter,
                "rollback suffix beginning at feature {} is not dependency closed",
                feature.0
            ),
            Self::SelectionConflict(feature, producer) => write!(
                formatter,
                "selected feature {} does not produce selected subshape from feature {}",
                feature.0, producer.0
            ),
            Self::SubshapeWrongDocument => {
                formatter.write_str("selected subshape belongs to another document")
            }
            Self::SubshapeWrongDefinition(expected, actual) => write!(
                formatter,
                "selected subshape belongs to definition {}, expected {}",
                actual.0, expected.0
            ),
            Self::SubshapeProducerHasNoBody(feature) => write!(
                formatter,
                "selected subshape producer {} has no output body",
                feature.0
            ),
            Self::SubshapeAmbiguous(count) => {
                write!(
                    formatter,
                    "selected subshape is ambiguous across {count} candidates"
                )
            }
            Self::SubshapeLost => formatter.write_str("selected subshape is lost"),
            Self::SubshapeQuarantined(reason) => {
                write!(formatter, "selected subshape is quarantined: {reason:?}")
            }
            Self::InvalidFeatureGraph => formatter.write_str("feature graph is invalid"),
        }
    }
}

impl std::error::Error for FeatureHistoryError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyHistoryMutation {
    SuppressFrom(FeatureId),
    Resume,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BodyHistoryMutationRequest {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub mutation: BodyHistoryMutation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyHistoryMutationPreview {
    pub source_revision: u64,
    pub source_digest: String,
    pub body_id: BodyId,
    pub suppressed_feature_ids: Vec<FeatureId>,
    pub affected_feature_ids: Vec<FeatureId>,
    pub unchanged_body_ids: Vec<BodyId>,
    pub proposal: Proposal,
}

#[derive(Debug, PartialEq)]
pub enum BodyHistoryMutationError {
    History(FeatureHistoryError),
    NoSuppressedSuffix(BodyId),
    Unchanged(BodyId),
    Proposal(ProposalPrepareError),
}

impl fmt::Display for BodyHistoryMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::History(error) => error.fmt(formatter),
            Self::NoSuppressedSuffix(body) => {
                write!(
                    formatter,
                    "body {} has no suppressed suffix to resume",
                    body.0
                )
            }
            Self::Unchanged(body) => {
                write!(
                    formatter,
                    "body {} already has the requested suppressed suffix",
                    body.0
                )
            }
            Self::Proposal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BodyHistoryMutationError {}

pub fn prepare_body_history_mutation(
    document: &DocumentStore,
    request: BodyHistoryMutationRequest,
    principal: ProposalPrincipal,
) -> Result<BodyHistoryMutationPreview, BodyHistoryMutationError> {
    let snapshot = document.current();
    let definition =
        snapshot
            .definition(request.definition_id)
            .ok_or(BodyHistoryMutationError::History(
                FeatureHistoryError::DefinitionNotFound(request.definition_id),
            ))?;
    if definition.body(request.body_id).is_none() {
        return Err(BodyHistoryMutationError::History(
            FeatureHistoryError::BodyNotFound(request.definition_id, request.body_id),
        ));
    }
    let graph = snapshot
        .feature_dependency_graph()
        .map_err(|_| BodyHistoryMutationError::History(FeatureHistoryError::InvalidFeatureGraph))?;
    let topological_order = graph
        .topological_order()
        .iter()
        .copied()
        .filter(|id| {
            snapshot
                .feature(*id)
                .is_some_and(|feature| feature.definition_id() == request.definition_id)
        })
        .collect::<Vec<_>>();
    let mut body_feature_ids = BTreeMap::new();
    for body in definition.bodies() {
        body_feature_ids.insert(
            body.id(),
            collect_body_history(
                &snapshot,
                request.definition_id,
                body.id(),
                &graph,
                &topological_order,
            )
            .map_err(BodyHistoryMutationError::History)?,
        );
    }
    let current = snapshot
        .suppressed_feature_ids(request.definition_id, request.body_id)
        .cloned()
        .unwrap_or_default();
    let suppressed_feature_ids = match request.mutation {
        BodyHistoryMutation::SuppressFrom(first_suppressed_feature_id) => {
            build_rollback_preview(
                request.definition_id,
                RollbackPreviewRequest {
                    body_id: request.body_id,
                    first_suppressed_feature_id,
                },
                &graph,
                &topological_order,
                &body_feature_ids,
            )
            .map_err(BodyHistoryMutationError::History)?
            .suppressed_feature_ids
        }
        BodyHistoryMutation::Resume => {
            if current.is_empty() {
                return Err(BodyHistoryMutationError::NoSuppressedSuffix(
                    request.body_id,
                ));
            }
            Vec::new()
        }
    };
    let requested = suppressed_feature_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if current == requested {
        return Err(BodyHistoryMutationError::Unchanged(request.body_id));
    }
    let affected_feature_ids = graph
        .dependent_closure(current.iter().chain(&requested).copied())
        .into_iter()
        .collect::<Vec<_>>();
    let context = ProposalContext {
        principal,
        goal: ProposalGoal::CanonicalPreview,
        assumptions: affected_feature_ids
            .iter()
            .copied()
            .map(|id| ProposalAssumption::TargetExists(AuthoritativeDependency::Feature(id)))
            .collect(),
        risk: ProposalRisk::Standard,
        confirmation: ProposalConfirmation::ReviewRequired,
        requested_budget: crate::document::ProposalBudget::HOST_MAX,
    };
    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::SetBodyFeatureSuppression {
                definition_id: request.definition_id,
                body_id: request.body_id,
                suppressed_feature_ids: suppressed_feature_ids.clone(),
            }]),
            context,
        )
        .map_err(BodyHistoryMutationError::Proposal)?;
    Ok(BodyHistoryMutationPreview {
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        body_id: request.body_id,
        suppressed_feature_ids,
        affected_feature_ids,
        unchanged_body_ids: definition
            .bodies()
            .filter_map(|body| (body.id() != request.body_id).then_some(body.id()))
            .collect(),
        proposal,
    })
}

pub fn project_feature_history(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    definition_id: DefinitionId,
    query: &FeatureHistoryQuery,
) -> Result<FeatureHistoryProjection, FeatureHistoryError> {
    let definition = snapshot
        .definition(definition_id)
        .ok_or(FeatureHistoryError::DefinitionNotFound(definition_id))?;
    let graph = snapshot
        .feature_dependency_graph()
        .map_err(|_| FeatureHistoryError::InvalidFeatureGraph)?;
    let definition_features = definition
        .feature_ids()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let topological_order = graph
        .topological_order()
        .iter()
        .copied()
        .filter(|id| definition_features.contains(id))
        .collect::<Vec<_>>();
    let topological_indices = topological_order
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<BTreeMap<_, _>>();

    let mut body_feature_ids = BTreeMap::new();
    for body in definition.bodies() {
        body_feature_ids.insert(
            body.id(),
            collect_body_history(
                snapshot,
                definition_id,
                body.id(),
                &graph,
                &topological_order,
            )?,
        );
    }

    let rollback_preview = query
        .rollback_preview
        .map(|request| {
            build_rollback_preview(
                definition_id,
                request,
                &graph,
                &topological_order,
                &body_feature_ids,
            )
        })
        .transpose()?;
    let suppressed: BTreeSet<FeatureId> = rollback_preview
        .as_ref()
        .map(|preview| preview.suppressed_feature_ids.iter().copied().collect())
        .unwrap_or_default();

    let mut bodies = Vec::new();
    for body in definition.bodies() {
        let ids = &body_feature_ids[&body.id()];
        let features = topological_order
            .iter()
            .filter(|id| ids.contains(id))
            .map(|id| {
                let feature = snapshot
                    .feature(*id)
                    .ok_or(FeatureHistoryError::FeatureNotFound(*id))?;
                let ownership = definition
                    .feature_body_ownership(*id)
                    .ok_or(FeatureHistoryError::FeatureNotFound(*id))?;
                Ok(FeatureHistoryEntry {
                    feature_id: *id,
                    name: feature.name().to_owned(),
                    topological_index: topological_indices[id],
                    dependencies: graph
                        .dependencies(*id)
                        .unwrap_or(&BTreeSet::new())
                        .iter()
                        .copied()
                        .collect(),
                    dependents: graph
                        .dependents(*id)
                        .unwrap_or(&BTreeSet::new())
                        .iter()
                        .copied()
                        .collect(),
                    input_body_ids: ownership.input_body_ids().to_vec(),
                    output_body_id: ownership.output_body_id(),
                    state: if snapshot
                        .suppressed_feature_ids(definition_id, body.id())
                        .is_some_and(|canonical| canonical.contains(id))
                        || (rollback_preview
                            .as_ref()
                            .is_some_and(|preview| preview.body_id == body.id())
                            && suppressed.contains(id))
                    {
                        FeatureHistoryState::RollbackSuppressed
                    } else {
                        FeatureHistoryState::Active
                    },
                })
            })
            .collect::<Result<Vec<_>, FeatureHistoryError>>()?;
        bodies.push(BodyFeatureHistory {
            body_id: body.id(),
            name: body.name().to_owned(),
            visible: body.visible(),
            active: body.id() == definition.active_body_id(),
            consumed_by: body.consumed_by(),
            features,
        });
    }

    let selected_feature = query
        .selected_feature_id
        .map(|feature_id| {
            let feature = snapshot
                .feature(feature_id)
                .ok_or(FeatureHistoryError::FeatureNotFound(feature_id))?;
            if feature.definition_id() != definition_id {
                return Err(FeatureHistoryError::FeatureOutsideDefinition(
                    feature_id,
                    definition_id,
                ));
            }
            let ownership = definition
                .feature_body_ownership(feature_id)
                .ok_or(FeatureHistoryError::FeatureNotFound(feature_id))?;
            Ok(SelectedFeatureProvenance {
                feature_id,
                definition_id,
                body_ids: body_feature_ids
                    .iter()
                    .filter_map(|(body_id, ids)| ids.contains(&feature_id).then_some(*body_id))
                    .collect(),
                dependencies: graph
                    .dependencies(feature_id)
                    .unwrap_or(&BTreeSet::new())
                    .iter()
                    .copied()
                    .collect(),
                input_body_ids: ownership.input_body_ids().to_vec(),
                output_body_id: ownership.output_body_id(),
            })
        })
        .transpose()?;

    let selected_subshape = query
        .selected_subshape
        .as_ref()
        .map(|reference| {
            project_subshape_provenance(
                snapshot,
                exact_results,
                definition_id,
                query.selected_feature_id,
                reference,
            )
        })
        .transpose()?;

    Ok(FeatureHistoryProjection {
        revision_id: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        definition_id,
        bodies,
        selected_feature,
        selected_subshape,
        rollback_preview,
    })
}

fn collect_body_history(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    body_id: BodyId,
    graph: &FeatureDependencyGraph,
    topological_order: &[FeatureId],
) -> Result<BTreeSet<FeatureId>, FeatureHistoryError> {
    let definition = snapshot
        .definition(definition_id)
        .ok_or(FeatureHistoryError::DefinitionNotFound(definition_id))?;
    let mut history = BTreeSet::new();
    let mut pending = topological_order
        .iter()
        .copied()
        .filter(|id| {
            definition
                .feature_body_ownership(*id)
                .and_then(|ownership| ownership.output_body_id())
                == Some(body_id)
        })
        .collect::<Vec<_>>();
    while let Some(feature_id) = pending.pop() {
        if !history.insert(feature_id) {
            continue;
        }
        for dependency in graph
            .dependencies(feature_id)
            .ok_or(FeatureHistoryError::FeatureNotFound(feature_id))?
        {
            let ownership = definition
                .feature_body_ownership(*dependency)
                .ok_or(FeatureHistoryError::FeatureNotFound(*dependency))?;
            if ownership
                .output_body_id()
                .is_none_or(|output| output == body_id)
            {
                pending.push(*dependency);
            }
        }
    }
    Ok(history)
}

fn build_rollback_preview(
    definition_id: DefinitionId,
    request: RollbackPreviewRequest,
    graph: &FeatureDependencyGraph,
    topological_order: &[FeatureId],
    body_feature_ids: &BTreeMap<BodyId, BTreeSet<FeatureId>>,
) -> Result<RollbackPreview, FeatureHistoryError> {
    let body_history =
        body_feature_ids
            .get(&request.body_id)
            .ok_or(FeatureHistoryError::BodyNotFound(
                definition_id,
                request.body_id,
            ))?;
    let ordered_history = topological_order
        .iter()
        .copied()
        .filter(|id| body_history.contains(id))
        .collect::<Vec<_>>();
    let boundary = ordered_history
        .iter()
        .position(|id| *id == request.first_suppressed_feature_id)
        .ok_or(FeatureHistoryError::FeatureOutsideBody(
            request.first_suppressed_feature_id,
            request.body_id,
        ))?;
    let suppressed_feature_ids = ordered_history[boundary..].to_vec();
    let suppressed = suppressed_feature_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if suppressed.iter().any(|id| {
        graph.dependents(*id).is_some_and(|dependents| {
            dependents
                .iter()
                .any(|dependent| !suppressed.contains(dependent))
        })
    }) {
        return Err(FeatureHistoryError::RollbackNotDependencyClosed(
            request.first_suppressed_feature_id,
        ));
    }
    Ok(RollbackPreview {
        body_id: request.body_id,
        first_suppressed_feature_id: request.first_suppressed_feature_id,
        suppressed_feature_ids,
    })
}

fn project_subshape_provenance(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    definition_id: DefinitionId,
    selected_feature_id: Option<FeatureId>,
    reference: &BodySubshapeRef,
) -> Result<SelectedSubshapeProvenance, FeatureHistoryError> {
    if reference.document_id != snapshot.document_id() {
        return Err(FeatureHistoryError::SubshapeWrongDocument);
    }
    if reference.definition_id != definition_id {
        return Err(FeatureHistoryError::SubshapeWrongDefinition(
            definition_id,
            reference.definition_id,
        ));
    }
    if let Some(feature_id) = selected_feature_id
        && feature_id != reference.producer_feature_id
    {
        return Err(FeatureHistoryError::SelectionConflict(
            feature_id,
            reference.producer_feature_id,
        ));
    }
    let definition = snapshot
        .definition(definition_id)
        .ok_or(FeatureHistoryError::DefinitionNotFound(definition_id))?;
    let body_id = definition
        .feature_body_ownership(reference.producer_feature_id)
        .and_then(|ownership| ownership.output_body_id())
        .ok_or(FeatureHistoryError::SubshapeProducerHasNoBody(
            reference.producer_feature_id,
        ))?;
    let resolved = match exact_results.resolve_reference(snapshot, reference) {
        ExactReferenceResolution::Resolved { reference } => reference,
        ExactReferenceResolution::Ambiguous { candidate_count } => {
            return Err(FeatureHistoryError::SubshapeAmbiguous(candidate_count));
        }
        ExactReferenceResolution::Lost => return Err(FeatureHistoryError::SubshapeLost),
        ExactReferenceResolution::Quarantined { reason } => {
            return Err(FeatureHistoryError::SubshapeQuarantined(reason));
        }
    };
    Ok(SelectedSubshapeProvenance {
        body_id,
        profile_feature_id: resolved.profile_feature_id,
        producer_feature_id: resolved.producer_feature_id,
        semantic_role: resolved.semantic_role,
        source_element_id: resolved.source_element_id,
        lineage_digest: resolved.lineage_digest,
        result_fingerprint: resolved.result_fingerprint,
    })
}
