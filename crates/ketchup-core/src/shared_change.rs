#![forbid(unsafe_code)]

use crate::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyRecomputeStatus,
    AssemblyReferenceHealth, AssemblySolveStatus, AssemblySolverPolicy,
    recompute_rigid_assembly_mates_from_snapshot,
};
use crate::document::DocumentStore;
use crate::document::{
    AuthoritativeDependency, BodyId, CanonicalCommand, CloneDefinitionPlan, CommandBatch,
    DefinitionId, DocumentId, FeatureId, InstancePath, OccurrenceId, Proposal, ProposalAssumption,
    ProposalBudget, ProposalCommitError, ProposalConfirmation, ProposalContext, ProposalGoal,
    ProposalPrincipal, ProposalRisk, Snapshot, Transform,
};
use crate::drawing::{
    DrawingSheetId, DrawingSource, OrthographicDrawing, OrthographicViewKind,
    project_orthographic_drawing,
};
use crate::exact_product::{
    BodySubshapeRef, ExactBodyPackage, ExactFeatureChainRequest, ExactProductError,
    ExactReferenceResolution, ExactResultRegistry, canonical_reference_lineage_digest,
    exact_model_stl_export,
};
use crate::feature_history::{
    BodyHistoryMutationRequest, BodyParameterEditRequest, prepare_body_history_mutation,
    prepare_body_parameter_edit,
};
use std::fmt;
use std::sync::Arc;

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
    Cyclic,
    CrossDefinition(DefinitionId, DefinitionId),
    Unsupported(String),
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
            Self::Cyclic => formatter.write_str("feature dependency graph is cyclic"),
            Self::CrossDefinition(expected, actual) => write!(
                formatter,
                "occurrence definition {} does not match requested definition {}",
                expected.0, actual.0
            ),
            Self::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for OccurrenceForkImpactError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentReplacementImpactRequest {
    pub source_revision: u64,
    pub source_digest: String,
    pub target_document_id: DocumentId,
    pub selected_occurrence_id: OccurrenceId,
    pub target_definition_ids: Vec<DefinitionId>,
}

impl ComponentReplacementImpactRequest {
    #[must_use]
    pub fn new(
        snapshot: &Snapshot,
        selected_occurrence_id: OccurrenceId,
        target_definition_id: DefinitionId,
    ) -> Self {
        Self {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            target_document_id: snapshot.document_id(),
            selected_occurrence_id,
            target_definition_ids: vec![target_definition_id],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ComponentReplacementBodyCorrespondence {
    pub source_body_id: BodyId,
    pub target_body_id: BodyId,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ComponentReplacementFeatureCorrespondence {
    pub source_feature_id: FeatureId,
    pub target_feature_id: FeatureId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ComponentReplacementSubshapeCorrespondence {
    pub source_profile_feature_id: FeatureId,
    pub source_producer_feature_id: FeatureId,
    pub source_lineage_digest: String,
    pub target_profile_feature_id: FeatureId,
    pub target_producer_feature_id: FeatureId,
    pub target_lineage_digest: String,
    pub semantic_role: String,
    pub source_element_id: String,
    pub expected_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentReplacementMateReferenceImpact {
    pub mate_id: AssemblyMateId,
    pub occurrence_id: OccurrenceId,
    pub source_lineage_digest: String,
    pub target_lineage_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentReplacementImpactProjection {
    pub source_revision: u64,
    pub source_digest: String,
    pub candidate_digest: Option<String>,
    pub selected_occurrence_id: OccurrenceId,
    pub selected_instance_path: InstancePath,
    pub selected_transform: Transform,
    pub source_definition_id: DefinitionId,
    pub target_definition_id: DefinitionId,
    pub body_correspondence: Vec<ComponentReplacementBodyCorrespondence>,
    pub feature_correspondence: Vec<ComponentReplacementFeatureCorrespondence>,
    pub subshape_correspondence: Vec<ComponentReplacementSubshapeCorrespondence>,
    pub unchanged_source_occurrences: Vec<SharedChangeOccurrenceImpact>,
    pub unchanged_target_occurrences: Vec<SharedChangeOccurrenceImpact>,
    pub unchanged_definition_ids: Vec<DefinitionId>,
    pub exact_jobs: Vec<SharedChangeExactJob>,
    pub mate_references: Vec<ComponentReplacementMateReferenceImpact>,
    pub drawing_views: Vec<SharedChangeDrawingViewImpact>,
    pub exports: Vec<SharedChangeExportImpact>,
    pub proposal: Option<Proposal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentReplacementImpactError {
    Stale,
    CrossDocument(DocumentId, DocumentId),
    OccurrenceNotFound(OccurrenceId),
    TargetDefinitionNotFound(DefinitionId),
    SelfReplacement(DefinitionId),
    DuplicateTarget,
    Hidden(OccurrenceId),
    Failed(DefinitionId, BodyId),
    Ambiguous(DefinitionId, BodyId),
    Lost(AssemblyMateId),
    Cyclic,
    Incompatible(String),
    Unsupported(String),
}

impl fmt::Display for ComponentReplacementImpactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("component replacement impact request is stale"),
            Self::CrossDocument(expected, actual) => write!(
                formatter,
                "target document {} does not match source document {}",
                actual.0, expected.0
            ),
            Self::OccurrenceNotFound(id) => write!(formatter, "occurrence {} was not found", id.0),
            Self::TargetDefinitionNotFound(id) => {
                write!(formatter, "target definition {} was not found", id.0)
            }
            Self::SelfReplacement(id) => {
                write!(formatter, "definition {} cannot replace itself", id.0)
            }
            Self::DuplicateTarget => {
                formatter.write_str("component replacement requires exactly one target definition")
            }
            Self::Hidden(id) => write!(formatter, "occurrence {} is hidden", id.0),
            Self::Failed(definition_id, body_id) => write!(
                formatter,
                "definition {} body {} has no last-valid exact result",
                definition_id.0, body_id.0
            ),
            Self::Ambiguous(definition_id, body_id) => write!(
                formatter,
                "definition {} body {} has ambiguous exact results",
                definition_id.0, body_id.0
            ),
            Self::Lost(mate_id) => write!(
                formatter,
                "assembly mate {} has a lost exact reference",
                mate_id.0
            ),
            Self::Cyclic => formatter.write_str("feature dependency graph is cyclic"),
            Self::Incompatible(reason) | Self::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for ComponentReplacementImpactError {}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentReplacementCommitReceipt {
    pub revision_id: u64,
    pub canonical_digest: String,
    pub selected_occurrence_id: OccurrenceId,
    pub selected_instance_path: InstancePath,
    pub selected_transform: Transform,
    pub source_definition_id: DefinitionId,
    pub target_definition_id: DefinitionId,
    pub reused_target_results: Vec<(BodyId, String)>,
    pub rebound_mate_ids: Vec<AssemblyMateId>,
    pub drawings: Vec<OrthographicDrawing>,
    pub exports: Vec<SharedChangeExportImpact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentReplacementCommitError {
    Stale,
    InvalidImpact(String),
    ExactPublication(String),
    Commit(String),
}

impl fmt::Display for ComponentReplacementCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("component replacement impact is stale"),
            Self::InvalidImpact(reason) => formatter.write_str(reason),
            Self::ExactPublication(reason) => {
                write!(
                    formatter,
                    "component replacement exact publication failed: {reason}"
                )
            }
            Self::Commit(reason) => {
                write!(formatter, "component replacement commit failed: {reason}")
            }
        }
    }
}

impl std::error::Error for ComponentReplacementCommitError {}

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
    Dependency(String),
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

impl std::error::Error for SharedChangePropagationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharedChangeImpactError {
    Stale,
    DefinitionNotReused(DefinitionId),
    Failed(BodyId),
    Ambiguous(BodyId),
    Lost(AssemblyMateId),
    Cyclic,
    Unsupported(String),
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
            Self::Cyclic => formatter.write_str("feature dependency graph is cyclic"),
            Self::Unsupported(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for SharedChangeImpactError {}

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
    Dependency(String),
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

impl std::error::Error for OccurrenceForkPropagationError {}

pub fn commit_occurrence_fork_change<E>(
    document: &mut DocumentStore,
    exact_results: &mut ExactResultRegistry,
    impact: &OccurrenceForkImpactProjection,
    mut evaluate: impl FnMut(&ExactFeatureChainRequest) -> Result<Arc<ExactBodyPackage>, E>,
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
    if candidate_selected.definition_id() != impact.fork_definition_id
        || candidate_selected.name() != selected.name()
        || candidate_selected.transform() != selected.transform()
        || candidate_selected.parent() != selected.parent()
        || candidate_selected.tag() != selected.tag()
        || candidate_selected.visible() != selected.visible()
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork candidate changed more than the selected definition reference"
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
    let request = ExactFeatureChainRequest::from_snapshot_for_body(
        &candidate,
        impact.fork_definition_id,
        *body_id,
    )
    .map_err(|error| OccurrenceForkPropagationError::InvalidImpact(error.to_string()))?;
    if request.producer_feature_id() != job.producer_feature_id
        || request.canonical_input_digest != job.canonical_input_digest
    {
        return Err(OccurrenceForkPropagationError::InvalidImpact(
            "occurrence fork exact job changed".to_owned(),
        ));
    }

    let package = evaluate(&request)
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
        .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?;
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            return Err(OccurrenceForkPropagationError::Dependency(format!(
                "local rigid assembly is not publishable: {:?}",
                recomputed.status()
            )));
        }
        for mate in recomputed.mates() {
            if !direct_mate_ids.contains(&mate.id()) {
                continue;
            }
            let current = candidate.assembly_mate(mate.id()).ok_or_else(|| {
                OccurrenceForkPropagationError::Dependency(format!(
                    "assembly mate {} disappeared during local dependency staging",
                    mate.id().0
                ))
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
            .any(|(occurrence_id, _)| sibling_ids.contains(occurrence_id))
        {
            return Err(OccurrenceForkPropagationError::Dependency(
                "local rigid solve would move an unchanged source sibling".to_owned(),
            ));
        }
        if !transforms.is_empty() {
            commands.push(CanonicalCommand::ApplyAssemblySolve {
                source_revision: impact.source_revision,
                source_digest: impact.source_digest.clone(),
                transforms,
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
        .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?;
    let final_candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?;
    let final_results = ExactResultRegistry::carried_forward(&final_candidate, &staged_results);
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
            "occurrence fork changed the assembly mate set".to_owned(),
        ));
    }
    for source_mate in source.assembly_mates() {
        let candidate_mate = candidate.assembly_mate(source_mate.id()).ok_or_else(|| {
            OccurrenceForkPropagationError::Dependency(format!(
                "assembly mate {} disappeared during local dependency staging",
                source_mate.id().0
            ))
        })?;
        if !direct_mate_ids.contains(&source_mate.id()) {
            if candidate_mate != source_mate {
                return Err(OccurrenceForkPropagationError::Dependency(format!(
                    "unrelated assembly mate {} changed during occurrence fork",
                    source_mate.id().0
                )));
            }
            continue;
        }
        if candidate_mate.kind() != source_mate.kind() {
            return Err(OccurrenceForkPropagationError::Dependency(format!(
                "assembly mate {} changed kind during occurrence fork",
                source_mate.id().0
            )));
        }
        for (before, after) in [source_mate.endpoint_a(), source_mate.endpoint_b()]
            .into_iter()
            .zip([candidate_mate.endpoint_a(), candidate_mate.endpoint_b()])
        {
            if before.occurrence_id() != impact.selected_occurrence_id && before != after {
                return Err(OccurrenceForkPropagationError::Dependency(format!(
                    "assembly mate {} changed its non-selected endpoint",
                    source_mate.id().0
                )));
            }
        }
    }

    for expected in &impact.mate_references {
        let mate = candidate.assembly_mate(expected.mate_id).ok_or_else(|| {
            OccurrenceForkPropagationError::Dependency(format!(
                "assembly mate {} disappeared during local dependency staging",
                expected.mate_id.0
            ))
        })?;
        let endpoint = [mate.endpoint_a(), mate.endpoint_b()]
            .into_iter()
            .find(|endpoint| {
                endpoint.occurrence_id() == expected.occurrence_id
                    && endpoint.reference().lineage_digest == expected.fork_lineage_digest
            })
            .ok_or_else(|| {
                OccurrenceForkPropagationError::Dependency(format!(
                    "assembly mate {} lost selected fork lineage",
                    expected.mate_id.0
                ))
            })?;
        if endpoint.health() != AssemblyReferenceHealth::Resolved
            || endpoint.reference().definition_id != expected.fork_definition_id
            || endpoint.reference().producer_feature_id != expected.fork_producer_feature_id
        {
            return Err(OccurrenceForkPropagationError::Dependency(format!(
                "assembly mate {} did not resolve to the selected fork",
                expected.mate_id.0
            )));
        }
        match exact_results.resolve_reference(candidate, endpoint.reference()) {
            ExactReferenceResolution::Resolved { reference }
                if reference.as_ref() == endpoint.reference() => {}
            resolution => {
                return Err(OccurrenceForkPropagationError::Dependency(format!(
                    "assembly mate {} fork reference is not uniquely current: {resolution:?}",
                    expected.mate_id.0
                )));
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
        let sheet = snapshot.drawing_sheet(affected.sheet_id).ok_or_else(|| {
            OccurrenceForkPropagationError::Dependency(format!(
                "drawing sheet {} disappeared during local dependency staging",
                affected.sheet_id.0
            ))
        })?;
        let drawing = project_orthographic_drawing(snapshot, exact_results, sheet)
            .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?;
        if !drawing.is_current(snapshot) {
            return Err(OccurrenceForkPropagationError::Dependency(format!(
                "drawing sheet {} did not refresh from current fork evidence",
                affected.sheet_id.0
            )));
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
                "occurrence fork export impact includes a non-selected branch".to_owned(),
            ));
        }
        let mut bodies = Vec::with_capacity(affected.occurrence_paths.len());
        for path in &affected.occurrence_paths {
            let occurrence = scene
                .iter()
                .find(|occurrence| occurrence.instance_path == *path && occurrence.visible)
                .ok_or_else(|| {
                    OccurrenceForkPropagationError::Dependency(format!(
                        "fork export occurrence path {path:?} is not visible and current"
                    ))
                })?;
            if occurrence.definition_id != impact.fork_definition_id {
                return Err(OccurrenceForkPropagationError::Dependency(
                    "fork export path does not resolve to the selected fork definition".to_owned(),
                ));
            }
            let package = exact_results
                .get_body(snapshot, occurrence.definition_id, body_id)
                .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?
                .ok_or_else(|| {
                    OccurrenceForkPropagationError::Dependency(format!(
                        "fork export occurrence path {path:?} has no current exact body"
                    ))
                })?;
            ExactFeatureChainRequest::from_snapshot_for_producer(
                snapshot,
                occurrence.definition_id,
                package.producer_feature_id(),
            )
            .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?;
            bodies.push((package.as_ref(), occurrence.transform));
        }
        if affected.format == SharedChangeExportFormat::Stl {
            exact_model_stl_export(snapshot, &bodies)
                .map_err(|error| OccurrenceForkPropagationError::Dependency(error.to_string()))?;
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
    mut evaluate: impl FnMut(&ExactFeatureChainRequest) -> Result<Arc<ExactBodyPackage>, E>,
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

    let [body_id] = impact.affected_body_ids.as_slice() else {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition change must affect exactly one body branch".to_owned(),
        ));
    };
    let [job] = impact.exact_jobs.as_slice() else {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition change must schedule exactly one exact body job".to_owned(),
        ));
    };
    if job.definition_id != impact.definition_id || job.body_id != *body_id {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition exact job does not match the affected body".to_owned(),
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
        .get_body(&source, impact.definition_id, *body_id)
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
        .preview_batch(impact.proposal.batch())
        .map_err(|error| SharedChangePropagationError::InvalidImpact(error.to_string()))?;
    if candidate.canonical_digest() != impact.candidate_digest {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition candidate digest changed".to_owned(),
        ));
    }
    let request = ExactFeatureChainRequest::from_snapshot_for_body(
        &candidate,
        impact.definition_id,
        *body_id,
    )
    .map_err(|error| SharedChangePropagationError::InvalidImpact(error.to_string()))?;
    if request.producer_feature_id() != job.producer_feature_id
        || request.canonical_input_digest != job.canonical_input_digest
    {
        return Err(SharedChangePropagationError::InvalidImpact(
            "shared-definition exact job changed".to_owned(),
        ));
    }

    let package = evaluate(&request)
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
        .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
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
        .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            return Err(SharedChangePropagationError::Dependency(format!(
                "rigid assembly is not publishable: {:?}",
                recomputed.status()
            )));
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
        .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
    let final_candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
    let final_results = ExactResultRegistry::carried_forward(&final_candidate, &staged_results);
    let final_package = final_results
        .get_body(&final_candidate, impact.definition_id, *body_id)
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
    let exports = refresh_export_eligibility(&final_candidate, &final_results, impact, *body_id)?;

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
        body_id: *body_id,
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
                SharedChangePropagationError::Dependency(format!(
                    "assembly mate {} lost semantic role {} during producer transition",
                    mate.id().0,
                    endpoint.reference().semantic_role
                ))
            })?;
            if matches.next().is_some() {
                return Err(SharedChangePropagationError::Dependency(format!(
                    "assembly mate {} has an ambiguous semantic role {} during producer transition",
                    mate.id().0,
                    endpoint.reference().semantic_role
                )));
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
        let mate = snapshot.assembly_mate(expected.mate_id).ok_or_else(|| {
            SharedChangePropagationError::Dependency(format!(
                "assembly mate {} disappeared during dependency staging",
                expected.mate_id.0
            ))
        })?;
        let endpoint = [mate.endpoint_a(), mate.endpoint_b()]
            .into_iter()
            .find(|endpoint| endpoint.occurrence_id() == expected.occurrence_id)
            .ok_or_else(|| {
                SharedChangePropagationError::Dependency(format!(
                    "assembly mate {} lost occurrence {}",
                    expected.mate_id.0, expected.occurrence_id.0
                ))
            })?;
        if endpoint.health() != AssemblyReferenceHealth::Resolved {
            return Err(SharedChangePropagationError::Dependency(format!(
                "assembly mate {} did not rebind to a resolved reference",
                expected.mate_id.0
            )));
        }
        match exact_results.resolve_reference(snapshot, endpoint.reference()) {
            ExactReferenceResolution::Resolved { reference }
                if reference.lineage_digest == expected.lineage_digest => {}
            ExactReferenceResolution::Resolved { reference }
                if producer_transition && reference.producer_feature_id == producer_feature_id => {}
            ExactReferenceResolution::Resolved { .. } => {
                return Err(SharedChangePropagationError::Dependency(format!(
                    "assembly mate {} changed stable subshape lineage",
                    expected.mate_id.0
                )));
            }
            resolution => {
                return Err(SharedChangePropagationError::Dependency(format!(
                    "assembly mate {} reference is not uniquely current: {resolution:?}",
                    expected.mate_id.0
                )));
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
        let sheet = snapshot.drawing_sheet(affected.sheet_id).ok_or_else(|| {
            SharedChangePropagationError::Dependency(format!(
                "drawing sheet {} disappeared during dependency staging",
                affected.sheet_id.0
            ))
        })?;
        let drawing = project_orthographic_drawing(snapshot, exact_results, sheet)
            .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
        if !drawing.is_current(snapshot) {
            return Err(SharedChangePropagationError::Dependency(format!(
                "drawing sheet {} did not refresh from current exact evidence",
                affected.sheet_id.0
            )));
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
                    SharedChangePropagationError::Dependency(format!(
                        "export occurrence path {path:?} is not visible and current"
                    ))
                })?;
            let package = exact_results
                .get_body(snapshot, occurrence.definition_id, body_id)
                .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?
                .ok_or_else(|| {
                    SharedChangePropagationError::Dependency(format!(
                        "export occurrence path {path:?} has no current exact body"
                    ))
                })?;
            ExactFeatureChainRequest::from_snapshot_for_producer(
                snapshot,
                occurrence.definition_id,
                package.producer_feature_id(),
            )
            .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
            bodies.push((package.as_ref(), occurrence.transform));
        }
        if affected.format == SharedChangeExportFormat::Stl {
            exact_model_stl_export(snapshot, &bodies)
                .map_err(|error| SharedChangePropagationError::Dependency(error.to_string()))?;
        }
        refreshed.push(SharedChangeExportImpact {
            format: affected.format,
            occurrence_paths: affected.occurrence_paths.clone(),
            eligibility: SharedChangeExportEligibility::CurrentExact,
        });
    }
    Ok(refreshed)
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

pub fn project_component_replacement_impact(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    request: ComponentReplacementImpactRequest,
) -> Result<ComponentReplacementImpactProjection, ComponentReplacementImpactError> {
    project_component_replacement_impact_for_principal(
        document,
        exact_results,
        request,
        ProposalPrincipal::ManualClient,
    )
}

pub fn project_component_replacement_impact_for_principal(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    request: ComponentReplacementImpactRequest,
    principal: ProposalPrincipal,
) -> Result<ComponentReplacementImpactProjection, ComponentReplacementImpactError> {
    let source = document.current();
    if source.revision_id() != request.source_revision
        || source.canonical_digest() != request.source_digest
    {
        return Err(ComponentReplacementImpactError::Stale);
    }
    if source.document_id() != request.target_document_id {
        return Err(ComponentReplacementImpactError::CrossDocument(
            source.document_id(),
            request.target_document_id,
        ));
    }
    source
        .feature_dependency_graph()
        .map_err(|_| ComponentReplacementImpactError::Cyclic)?;

    let [target_definition_id] = request.target_definition_ids.as_slice() else {
        return Err(ComponentReplacementImpactError::DuplicateTarget);
    };
    let selected = source.occurrence(request.selected_occurrence_id).ok_or(
        ComponentReplacementImpactError::OccurrenceNotFound(request.selected_occurrence_id),
    )?;
    let source_definition_id = selected.definition_id();
    if source_definition_id == *target_definition_id {
        return Err(ComponentReplacementImpactError::SelfReplacement(
            source_definition_id,
        ));
    }
    let selected_scene = source
        .scene_query()
        .into_iter()
        .find(|occurrence| {
            occurrence.occurrence_id == request.selected_occurrence_id
                && occurrence.instance_path.is_root()
        })
        .ok_or(ComponentReplacementImpactError::OccurrenceNotFound(
            request.selected_occurrence_id,
        ))?;
    if !selected_scene.visible {
        return Err(ComponentReplacementImpactError::Hidden(
            request.selected_occurrence_id,
        ));
    }

    let source_definition = source.definition(source_definition_id).ok_or_else(|| {
        ComponentReplacementImpactError::Unsupported(format!(
            "source definition {} was not found",
            source_definition_id.0
        ))
    })?;
    let target_definition = source.definition(*target_definition_id).ok_or(
        ComponentReplacementImpactError::TargetDefinitionNotFound(*target_definition_id),
    )?;
    if !source_definition.local_occurrence_ids().is_empty()
        || !source_definition.local_group_ids().is_empty()
        || !target_definition.local_occurrence_ids().is_empty()
        || !target_definition.local_group_ids().is_empty()
    {
        return Err(ComponentReplacementImpactError::Unsupported(
            "component replacement does not support nested definition instances".to_owned(),
        ));
    }

    let source_body_ids = source_definition
        .bodies()
        .map(|body| {
            if !body.visible() || body.consumed_by().is_some() {
                Err(ComponentReplacementImpactError::Unsupported(format!(
                    "source body {} is hidden or consumed",
                    body.id().0
                )))
            } else {
                Ok(body.id())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let target_body_ids = target_definition
        .bodies()
        .map(|body| {
            if !body.visible() || body.consumed_by().is_some() {
                Err(ComponentReplacementImpactError::Unsupported(format!(
                    "target body {} is hidden or consumed",
                    body.id().0
                )))
            } else {
                Ok(body.id())
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if source_body_ids.len() != target_body_ids.len() {
        return Err(ComponentReplacementImpactError::Incompatible(
            "source and target definitions have different body counts".to_owned(),
        ));
    }

    let mut body_correspondence = Vec::with_capacity(source_body_ids.len());
    let mut used_target_bodies = std::collections::BTreeSet::new();
    for source_body_id in source_body_ids {
        let source_package = replacement_current_body_result(
            exact_results,
            &source,
            source_definition_id,
            source_body_id,
        )?;
        let source_signature = replacement_body_signature(&source, source_package)?;
        let mut matches = Vec::new();
        for target_body_id in &target_body_ids {
            if used_target_bodies.contains(target_body_id) {
                continue;
            }
            let target_package = replacement_current_body_result(
                exact_results,
                &source,
                *target_definition_id,
                *target_body_id,
            )?;
            if replacement_body_signature(&source, target_package)? == source_signature {
                matches.push(*target_body_id);
            }
        }
        let [target_body_id] = matches.as_slice() else {
            return Err(ComponentReplacementImpactError::Incompatible(format!(
                "source body {} has no unique compatible target body",
                source_body_id.0
            )));
        };
        used_target_bodies.insert(*target_body_id);
        body_correspondence.push(ComponentReplacementBodyCorrespondence {
            source_body_id,
            target_body_id: *target_body_id,
        });
    }
    body_correspondence.sort_unstable();

    let graph = source
        .feature_dependency_graph()
        .map_err(|_| ComponentReplacementImpactError::Cyclic)?;
    let source_feature_ids = graph
        .topological_order()
        .iter()
        .copied()
        .filter(|feature_id| {
            source
                .feature(*feature_id)
                .is_some_and(|feature| feature.definition_id() == source_definition_id)
        })
        .collect::<Vec<_>>();
    let target_feature_ids = graph
        .topological_order()
        .iter()
        .copied()
        .filter(|feature_id| {
            source
                .feature(*feature_id)
                .is_some_and(|feature| feature.definition_id() == *target_definition_id)
        })
        .collect::<Vec<_>>();
    if source_feature_ids.len() != target_feature_ids.len() {
        return Err(ComponentReplacementImpactError::Incompatible(
            "source and target definitions have different feature counts".to_owned(),
        ));
    }

    let mut feature_correspondence = Vec::with_capacity(source_feature_ids.len());
    let mut used_target_features = std::collections::BTreeSet::new();
    for source_feature_id in source_feature_ids {
        let source_feature = source.feature(source_feature_id).ok_or_else(|| {
            ComponentReplacementImpactError::Unsupported(format!(
                "source feature {} was not found",
                source_feature_id.0
            ))
        })?;
        let source_dependencies = graph.dependencies(source_feature_id).ok_or_else(|| {
            ComponentReplacementImpactError::Unsupported(format!(
                "source feature {} has no dependency record",
                source_feature_id.0
            ))
        })?;
        let mapped_dependencies = source_dependencies
            .iter()
            .map(|dependency| {
                feature_correspondence
                    .iter()
                    .find_map(|mapping: &ComponentReplacementFeatureCorrespondence| {
                        (mapping.source_feature_id == *dependency)
                            .then_some(mapping.target_feature_id)
                    })
                    .ok_or_else(|| {
                        ComponentReplacementImpactError::Incompatible(format!(
                            "source feature {} dependency {} has no target mapping",
                            source_feature_id.0, dependency.0
                        ))
                    })
            })
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        let source_ownership = source_definition.feature_body_ownership(source_feature_id);
        let mapped_inputs = source_ownership
            .map(|ownership| {
                ownership
                    .input_body_ids()
                    .iter()
                    .map(|body_id| {
                        body_correspondence
                            .iter()
                            .find_map(|mapping| {
                                (mapping.source_body_id == *body_id)
                                    .then_some(mapping.target_body_id)
                            })
                            .ok_or_else(|| {
                                ComponentReplacementImpactError::Incompatible(format!(
                                    "source feature {} input body {} has no target mapping",
                                    source_feature_id.0, body_id.0
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let mapped_output = source_ownership
            .and_then(|ownership| ownership.output_body_id())
            .map(|body_id| {
                body_correspondence
                    .iter()
                    .find_map(|mapping| {
                        (mapping.source_body_id == body_id).then_some(mapping.target_body_id)
                    })
                    .ok_or_else(|| {
                        ComponentReplacementImpactError::Incompatible(format!(
                            "source feature {} output body {} has no target mapping",
                            source_feature_id.0, body_id.0
                        ))
                    })
            })
            .transpose()?;

        let matches = target_feature_ids
            .iter()
            .copied()
            .filter(|target_feature_id| !used_target_features.contains(target_feature_id))
            .filter(|target_feature_id| {
                let Some(target_feature) = source.feature(*target_feature_id) else {
                    return false;
                };
                if std::mem::discriminant(source_feature.kind())
                    != std::mem::discriminant(target_feature.kind())
                    || source.feature_is_suppressed(source_feature_id)
                        != source.feature_is_suppressed(*target_feature_id)
                {
                    return false;
                }
                if graph.dependencies(*target_feature_id) != Some(&mapped_dependencies) {
                    return false;
                }
                match (
                    mapped_inputs.as_ref(),
                    mapped_output,
                    target_definition.feature_body_ownership(*target_feature_id),
                ) {
                    (None, None, None) => true,
                    (Some(inputs), output, Some(ownership)) => {
                        inputs == ownership.input_body_ids() && output == ownership.output_body_id()
                    }
                    _ => false,
                }
            })
            .collect::<Vec<_>>();
        let [target_feature_id] = matches.as_slice() else {
            return Err(ComponentReplacementImpactError::Incompatible(format!(
                "source feature {} has no unique compatible target feature",
                source_feature_id.0
            )));
        };
        used_target_features.insert(*target_feature_id);
        feature_correspondence.push(ComponentReplacementFeatureCorrespondence {
            source_feature_id,
            target_feature_id: *target_feature_id,
        });
    }
    feature_correspondence.sort_unstable();

    let mut subshape_correspondence = Vec::new();
    let mut exact_jobs = Vec::with_capacity(body_correspondence.len());
    for body_mapping in &body_correspondence {
        let source_package = replacement_current_body_result(
            exact_results,
            &source,
            source_definition_id,
            body_mapping.source_body_id,
        )?;
        let target_package = replacement_current_body_result(
            exact_results,
            &source,
            *target_definition_id,
            body_mapping.target_body_id,
        )?;
        for source_reference in source_package.references() {
            let mapped_profile = feature_correspondence
                .iter()
                .find_map(|mapping| {
                    (mapping.source_feature_id == source_reference.profile_feature_id)
                        .then_some(mapping.target_feature_id)
                })
                .ok_or_else(|| {
                    ComponentReplacementImpactError::Incompatible(format!(
                        "source subshape {} has no target profile feature",
                        source_reference.semantic_role
                    ))
                })?;
            let mapped_producer = feature_correspondence
                .iter()
                .find_map(|mapping| {
                    (mapping.source_feature_id == source_reference.producer_feature_id)
                        .then_some(mapping.target_feature_id)
                })
                .ok_or_else(|| {
                    ComponentReplacementImpactError::Incompatible(format!(
                        "source subshape {} has no target producer feature",
                        source_reference.semantic_role
                    ))
                })?;
            let matches = target_package
                .references()
                .iter()
                .filter(|target_reference| {
                    target_reference.profile_feature_id == mapped_profile
                        && target_reference.producer_feature_id == mapped_producer
                        && target_reference.semantic_role == source_reference.semantic_role
                        && target_reference.source_element_id == source_reference.source_element_id
                        && target_reference.expected_type == source_reference.expected_type
                })
                .collect::<Vec<_>>();
            let [target_reference] = matches.as_slice() else {
                return Err(ComponentReplacementImpactError::Incompatible(format!(
                    "source subshape {} has no unique target subshape",
                    source_reference.semantic_role
                )));
            };
            subshape_correspondence.push(ComponentReplacementSubshapeCorrespondence {
                source_profile_feature_id: source_reference.profile_feature_id,
                source_producer_feature_id: source_reference.producer_feature_id,
                source_lineage_digest: source_reference.lineage_digest.clone(),
                target_profile_feature_id: target_reference.profile_feature_id,
                target_producer_feature_id: target_reference.producer_feature_id,
                target_lineage_digest: target_reference.lineage_digest.clone(),
                semantic_role: source_reference.semantic_role.clone(),
                source_element_id: source_reference.source_element_id.clone(),
                expected_type: source_reference.expected_type.clone(),
            });
        }
        if source_package.references().len() != target_package.references().len() {
            return Err(ComponentReplacementImpactError::Incompatible(format!(
                "source body {} and target body {} have different subshape counts",
                body_mapping.source_body_id.0, body_mapping.target_body_id.0
            )));
        }
        let exact_request = ExactFeatureChainRequest::from_snapshot_for_producer(
            &source,
            *target_definition_id,
            target_package.producer_feature_id(),
        )
        .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
        exact_jobs.push(SharedChangeExactJob {
            definition_id: *target_definition_id,
            body_id: body_mapping.target_body_id,
            producer_feature_id: target_package.producer_feature_id(),
            canonical_input_digest: exact_request.canonical_input_digest,
            last_valid_result_fingerprint: target_package.result_key().result_fingerprint.clone(),
        });
    }
    subshape_correspondence.sort_by(|left, right| {
        (
            left.source_profile_feature_id,
            left.source_producer_feature_id,
            left.target_profile_feature_id,
            left.target_producer_feature_id,
            &left.semantic_role,
            &left.source_element_id,
            &left.expected_type,
        )
            .cmp(&(
                right.source_profile_feature_id,
                right.source_producer_feature_id,
                right.target_profile_feature_id,
                right.target_producer_feature_id,
                &right.semantic_role,
                &right.source_element_id,
                &right.expected_type,
            ))
    });
    exact_jobs.sort_by_key(|job| (job.definition_id, job.body_id, job.producer_feature_id));

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
            if !matches!(
                mate.kind(),
                crate::assembly::AssemblyMateKind::CoincidentPlanar { .. }
                    | crate::assembly::AssemblyMateKind::ConcentricAxial { .. }
            ) {
                return Err(ComponentReplacementImpactError::Unsupported(format!(
                    "assembly mate {} is not planar or axial",
                    mate.id().0
                )));
            }
            if endpoint.reference().definition_id != source_definition_id {
                return Err(ComponentReplacementImpactError::Incompatible(format!(
                    "assembly mate {} does not reference the selected source definition",
                    mate.id().0
                )));
            }
            match endpoint.health() {
                AssemblyReferenceHealth::Resolved => {}
                AssemblyReferenceHealth::Ambiguous { .. } => {
                    return Err(ComponentReplacementImpactError::Ambiguous(
                        source_definition_id,
                        source_definition.active_body_id(),
                    ));
                }
                AssemblyReferenceHealth::Lost => {
                    return Err(ComponentReplacementImpactError::Lost(mate.id()));
                }
                AssemblyReferenceHealth::Broken => {
                    return Err(ComponentReplacementImpactError::Unsupported(format!(
                        "assembly mate {} has a broken exact reference",
                        mate.id().0
                    )));
                }
            }
            match exact_results.resolve_reference(&source, endpoint.reference()) {
                ExactReferenceResolution::Resolved { .. } => {}
                ExactReferenceResolution::Ambiguous { .. } => {
                    return Err(ComponentReplacementImpactError::Ambiguous(
                        source_definition_id,
                        source_definition.active_body_id(),
                    ));
                }
                ExactReferenceResolution::Lost => {
                    return Err(ComponentReplacementImpactError::Lost(mate.id()));
                }
                ExactReferenceResolution::Quarantined { reason } => {
                    return Err(ComponentReplacementImpactError::Unsupported(format!(
                        "assembly mate {} exact reference is quarantined: {reason:?}",
                        mate.id().0
                    )));
                }
            }
            let mapping = subshape_correspondence
                .iter()
                .find(|mapping| {
                    mapping.source_profile_feature_id == endpoint.reference().profile_feature_id
                        && mapping.source_producer_feature_id
                            == endpoint.reference().producer_feature_id
                        && mapping.source_lineage_digest == endpoint.reference().lineage_digest
                })
                .ok_or_else(|| {
                    ComponentReplacementImpactError::Incompatible(format!(
                        "assembly mate {} has no target subshape correspondence",
                        mate.id().0
                    ))
                })?;
            mate_references.push(ComponentReplacementMateReferenceImpact {
                mate_id: mate.id(),
                occurrence_id: request.selected_occurrence_id,
                source_lineage_digest: mapping.source_lineage_digest.clone(),
                target_lineage_digest: mapping.target_lineage_digest.clone(),
            });
            let target_reference = replacement_target_reference(
                exact_results,
                &source,
                *target_definition_id,
                &body_correspondence,
                mapping,
            )?;
            rebound_endpoints.push(AssemblyMateEndpoint::resolved(
                request.selected_occurrence_id,
                target_reference,
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

    let mut drawing_views = Vec::new();
    for sheet in source.drawing_sheets() {
        if matches!(
            sheet.source(),
            DrawingSource::RigidAssembly { occurrence_ids }
                if occurrence_ids.contains(&request.selected_occurrence_id)
        ) {
            let drawing = project_orthographic_drawing(&source, exact_results, sheet)
                .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
            if !drawing.is_current(&source) {
                return Err(ComponentReplacementImpactError::Stale);
            }
            for view in [
                OrthographicViewKind::Front,
                OrthographicViewKind::Top,
                OrthographicViewKind::Right,
            ] {
                drawing_views.push(SharedChangeDrawingViewImpact {
                    sheet_id: sheet.id(),
                    view,
                });
            }
        }
    }
    drawing_views.sort_unstable();

    let target_export_bodies = body_correspondence
        .iter()
        .map(|mapping| {
            replacement_current_body_result(
                exact_results,
                &source,
                *target_definition_id,
                mapping.target_body_id,
            )
            .map(|package| (package.as_ref(), selected_scene.transform))
        })
        .collect::<Result<Vec<_>, _>>()?;
    exact_model_stl_export(&source, &target_export_bodies)
        .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
    let exports = [
        SharedChangeExportFormat::Step,
        SharedChangeExportFormat::Stl,
    ]
    .into_iter()
    .map(|format| SharedChangeExportImpact {
        format,
        occurrence_paths: vec![selected_scene.instance_path.clone()],
        eligibility: SharedChangeExportEligibility::CurrentExact,
    })
    .collect();

    let mut unchanged_source_occurrences = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.instance_path.is_root()
                && occurrence.definition_id == source_definition_id
                && occurrence.occurrence_id != request.selected_occurrence_id
        })
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path,
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();
    unchanged_source_occurrences
        .sort_by(|left, right| left.instance_path.cmp(&right.instance_path));
    let mut unchanged_target_occurrences = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.instance_path.is_root() && occurrence.definition_id == *target_definition_id
        })
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path,
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();
    unchanged_target_occurrences
        .sort_by(|left, right| left.instance_path.cmp(&right.instance_path));

    let unchanged_definition_ids = source
        .definitions()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();
    let mut commands = vec![CanonicalCommand::RepointOccurrence {
        id: request.selected_occurrence_id,
        definition_id: *target_definition_id,
    }];
    commands.extend(mate_rebind_commands);
    let direct_mate_ids = mate_references
        .iter()
        .map(|reference| reference.mate_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut solved_transform_ids = std::collections::BTreeSet::new();
    if !direct_mate_ids.is_empty() {
        let rebound_candidate = source
            .preview_batch(&CommandBatch::new(commands.clone()))
            .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
        let rebound_results =
            ExactResultRegistry::carried_forward(&rebound_candidate, exact_results);
        let affected_mate_ids = dependent_mate_component(&rebound_candidate, &direct_mate_ids);
        let recomputed = recompute_rigid_assembly_mates_from_snapshot(
            &rebound_candidate,
            &rebound_results,
            AssemblySolverPolicy::default(),
            &affected_mate_ids,
        )
        .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
        let solve = recomputed.solve().ok_or_else(|| {
            ComponentReplacementImpactError::Unsupported(
                "component replacement local rigid solve produced no result".to_owned(),
            )
        })?;
        if recomputed.status() != AssemblyRecomputeStatus::Solved
            || solve.status() != AssemblySolveStatus::FullyConstrained
            || !solve.conflicting_mate_ids().is_empty()
            || !solve.maximum_residual().is_finite()
        {
            return Err(ComponentReplacementImpactError::Unsupported(format!(
                "component replacement local rigid solve is not fully constrained: {:?}/{:?}",
                recomputed.status(),
                solve.status()
            )));
        }
        let protected_occurrence_ids = unchanged_source_occurrences
            .iter()
            .chain(&unchanged_target_occurrences)
            .map(|occurrence| occurrence.occurrence_id)
            .chain(std::iter::once(request.selected_occurrence_id))
            .collect::<std::collections::BTreeSet<_>>();
        let mut transforms = solve
            .occurrences()
            .iter()
            .filter(|occurrence| !occurrence.grounded())
            .filter_map(|occurrence| {
                rebound_candidate
                    .occurrence(occurrence.occurrence_id())
                    .filter(|current| current.transform() != occurrence.transform())
                    .map(|_| (occurrence.occurrence_id(), occurrence.transform()))
            })
            .collect::<Vec<_>>();
        transforms.sort_by_key(|(occurrence_id, _)| *occurrence_id);
        if transforms
            .iter()
            .any(|(occurrence_id, _)| protected_occurrence_ids.contains(occurrence_id))
        {
            return Err(ComponentReplacementImpactError::Unsupported(
                "component replacement local rigid solve would move the selected or an unchanged source/target occurrence".to_owned(),
            ));
        }
        solved_transform_ids.extend(transforms.iter().map(|(occurrence_id, _)| *occurrence_id));
        if !transforms.is_empty() {
            commands.push(CanonicalCommand::ApplyAssemblySolve {
                source_revision: source.revision_id(),
                source_digest: source.canonical_digest(),
                transforms,
            });
        }
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
                    ProposalAssumption::TargetExists(AuthoritativeDependency::Definition(
                        *target_definition_id,
                    )),
                ],
                risk: ProposalRisk::Standard,
                confirmation: ProposalConfirmation::ReviewRequired,
                requested_budget: ProposalBudget::HOST_MAX,
            },
        )
        .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
    let candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
    let candidate_results = ExactResultRegistry::carried_forward(&candidate, exact_results);
    let candidate_selected = candidate.occurrence(request.selected_occurrence_id).ok_or(
        ComponentReplacementImpactError::OccurrenceNotFound(request.selected_occurrence_id),
    )?;
    if candidate_selected.definition_id() != *target_definition_id
        || candidate_selected.name() != selected.name()
        || candidate_selected.transform() != selected.transform()
        || candidate_selected.parent() != selected.parent()
        || candidate_selected.tag() != selected.tag()
        || candidate_selected.visible() != selected.visible()
        || candidate.occurrence_is_grounded(request.selected_occurrence_id)
            != source.occurrence_is_grounded(request.selected_occurrence_id)
    {
        return Err(ComponentReplacementImpactError::Unsupported(
            "component replacement candidate changed more than the selected definition reference"
                .to_owned(),
        ));
    }
    for occurrence in source
        .occurrences()
        .filter(|occurrence| occurrence.id() != request.selected_occurrence_id)
    {
        if !solved_transform_ids.contains(&occurrence.id())
            && candidate.occurrence(occurrence.id()) != Some(occurrence)
        {
            return Err(ComponentReplacementImpactError::Unsupported(
                "component replacement candidate changed an unrelated occurrence".to_owned(),
            ));
        }
    }
    for definition_id in &unchanged_definition_ids {
        if candidate.definition(*definition_id) != source.definition(*definition_id) {
            return Err(ComponentReplacementImpactError::Unsupported(
                "component replacement candidate changed a source or target definition".to_owned(),
            ));
        }
    }
    for sheet in source.drawing_sheets().filter(|sheet| {
        matches!(
            sheet.source(),
            DrawingSource::RigidAssembly { occurrence_ids }
                if occurrence_ids.contains(&request.selected_occurrence_id)
        )
    }) {
        project_orthographic_drawing(&candidate, &candidate_results, sheet)
            .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
    }
    let candidate_export_bodies = body_correspondence
        .iter()
        .map(|mapping| {
            replacement_current_body_result(
                &candidate_results,
                &candidate,
                *target_definition_id,
                mapping.target_body_id,
            )
            .map(|package| (package.as_ref(), selected_scene.transform))
        })
        .collect::<Result<Vec<_>, _>>()?;
    exact_model_stl_export(&candidate, &candidate_export_bodies)
        .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
    let candidate_digest = Some(candidate.canonical_digest());
    let proposal = Some(proposal);

    Ok(ComponentReplacementImpactProjection {
        source_revision: source.revision_id(),
        source_digest: source.canonical_digest(),
        candidate_digest,
        selected_occurrence_id: request.selected_occurrence_id,
        selected_instance_path: selected_scene.instance_path,
        selected_transform: selected_scene.transform,
        source_definition_id,
        target_definition_id: *target_definition_id,
        body_correspondence,
        feature_correspondence,
        subshape_correspondence,
        unchanged_source_occurrences,
        unchanged_target_occurrences,
        unchanged_definition_ids,
        exact_jobs,
        mate_references,
        drawing_views,
        exports,
        proposal,
    })
}

pub fn commit_component_replacement(
    document: &mut DocumentStore,
    exact_results: &mut ExactResultRegistry,
    impact: &ComponentReplacementImpactProjection,
) -> Result<ComponentReplacementCommitReceipt, ComponentReplacementCommitError> {
    let source = document.current();
    if source.revision_id() != impact.source_revision
        || source.canonical_digest() != impact.source_digest
    {
        return Err(ComponentReplacementCommitError::Stale);
    }
    let proposal = impact.proposal.as_ref().ok_or_else(|| {
        ComponentReplacementCommitError::InvalidImpact(
            "component replacement has no reviewed atomic proposal".to_owned(),
        )
    })?;
    if proposal.provenance_revision() != impact.source_revision
        || proposal.provenance_digest() != impact.source_digest
    {
        return Err(ComponentReplacementCommitError::Stale);
    }
    if !matches!(
        proposal.confirmation(),
        ProposalConfirmation::ReviewRequired
    ) {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement was not reviewed".to_owned(),
        ));
    }

    let refreshed = project_component_replacement_impact_for_principal(
        document,
        exact_results,
        ComponentReplacementImpactRequest::new(
            &source,
            impact.selected_occurrence_id,
            impact.target_definition_id,
        ),
        proposal.principal(),
    )
    .map_err(|error| match error {
        ComponentReplacementImpactError::Stale => ComponentReplacementCommitError::Stale,
        error => ComponentReplacementCommitError::InvalidImpact(error.to_string()),
    })?;
    if refreshed != *impact {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement impact no longer matches the complete current correspondence"
                .to_owned(),
        ));
    }

    let expected_mate_ids = impact
        .mate_references
        .iter()
        .map(|reference| reference.mate_id)
        .collect::<std::collections::BTreeSet<_>>();
    let protected_occurrence_ids = impact
        .unchanged_source_occurrences
        .iter()
        .chain(&impact.unchanged_target_occurrences)
        .map(|occurrence| occurrence.occurrence_id)
        .chain(std::iter::once(impact.selected_occurrence_id))
        .collect::<std::collections::BTreeSet<_>>();
    let mut repoint_count = 0;
    let mut rebound_mate_ids = std::collections::BTreeSet::new();
    let mut solve_count = 0;
    for command in proposal.batch().commands() {
        match command {
            CanonicalCommand::RepointOccurrence { id, definition_id }
                if *id == impact.selected_occurrence_id
                    && *definition_id == impact.target_definition_id =>
            {
                repoint_count += 1;
            }
            CanonicalCommand::RebindAssemblyMate(rebound)
                if expected_mate_ids.contains(&rebound.id()) =>
            {
                let source_mate = source.assembly_mate(rebound.id()).ok_or_else(|| {
                    ComponentReplacementCommitError::InvalidImpact(format!(
                        "component replacement introduced assembly mate {}",
                        rebound.id().0
                    ))
                })?;
                if rebound.kind() != source_mate.kind() {
                    return Err(ComponentReplacementCommitError::InvalidImpact(format!(
                        "component replacement changed assembly mate {} kind",
                        rebound.id().0
                    )));
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
                                || after.reference().definition_id != impact.target_definition_id
                                || !impact.mate_references.iter().any(|expected| {
                                    expected.mate_id == rebound.id()
                                        && expected.target_lineage_digest
                                            == after.reference().lineage_digest
                                })))
                    {
                        return Err(ComponentReplacementCommitError::InvalidImpact(format!(
                            "component replacement changed an unrelated endpoint of assembly mate {}",
                            rebound.id().0
                        )));
                    }
                }
                rebound_mate_ids.insert(rebound.id());
            }
            CanonicalCommand::ApplyAssemblySolve { transforms, .. }
                if transforms.iter().all(|(occurrence_id, _)| {
                    !protected_occurrence_ids.contains(occurrence_id)
                }) =>
            {
                solve_count += 1;
            }
            _ => {
                return Err(ComponentReplacementCommitError::InvalidImpact(
                    "component replacement proposal contains an unrelated canonical command"
                        .to_owned(),
                ));
            }
        }
    }
    if repoint_count != 1 || solve_count > 1 || rebound_mate_ids != expected_mate_ids {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement proposal does not match the reviewed repoint and dependency rebind"
                .to_owned(),
        ));
    }

    let selected = source
        .occurrence(impact.selected_occurrence_id)
        .ok_or_else(|| {
            ComponentReplacementCommitError::InvalidImpact(
                "selected occurrence disappeared from the source snapshot".to_owned(),
            )
        })?;
    if selected.definition_id() != impact.source_definition_id {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "selected occurrence no longer uses the reviewed source definition".to_owned(),
        ));
    }
    let expected_definition_ids = source
        .definitions()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();
    if expected_definition_ids != impact.unchanged_definition_ids {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement definition-isolation evidence is incomplete".to_owned(),
        ));
    }

    let candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| ComponentReplacementCommitError::InvalidImpact(error.to_string()))?;
    if impact.candidate_digest.as_deref() != Some(candidate.canonical_digest().as_str()) {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement candidate digest changed".to_owned(),
        ));
    }
    let candidate_selected = candidate
        .occurrence(impact.selected_occurrence_id)
        .ok_or_else(|| {
            ComponentReplacementCommitError::InvalidImpact(
                "component replacement candidate lost the selected occurrence".to_owned(),
            )
        })?;
    if candidate_selected.definition_id() != impact.target_definition_id
        || candidate_selected.name() != selected.name()
        || candidate_selected.transform() != selected.transform()
        || candidate_selected.parent() != selected.parent()
        || candidate_selected.tag() != selected.tag()
        || candidate_selected.visible() != selected.visible()
        || candidate.occurrence_is_grounded(impact.selected_occurrence_id)
            != source.occurrence_is_grounded(impact.selected_occurrence_id)
    {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement changed more than the selected definition reference".to_owned(),
        ));
    }
    let solved_transform_ids = proposal
        .batch()
        .commands()
        .iter()
        .find_map(|command| match command {
            CanonicalCommand::ApplyAssemblySolve { transforms, .. } => Some(
                transforms
                    .iter()
                    .map(|(occurrence_id, _)| *occurrence_id)
                    .collect::<std::collections::BTreeSet<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    for occurrence in source
        .occurrences()
        .filter(|occurrence| occurrence.id() != impact.selected_occurrence_id)
    {
        if !solved_transform_ids.contains(&occurrence.id())
            && candidate.occurrence(occurrence.id()) != Some(occurrence)
        {
            return Err(ComponentReplacementCommitError::InvalidImpact(
                "component replacement changed an unrelated occurrence".to_owned(),
            ));
        }
    }
    for definition_id in &expected_definition_ids {
        if candidate.definition(*definition_id) != source.definition(*definition_id) {
            return Err(ComponentReplacementCommitError::InvalidImpact(
                "component replacement changed a source or target definition".to_owned(),
            ));
        }
    }

    let staged_results = ExactResultRegistry::carried_forward(&candidate, exact_results);
    if staged_results.values().count() != exact_results.values().count() {
        return Err(ComponentReplacementCommitError::ExactPublication(
            "an unchanged exact result could not be carried to the replacement revision".to_owned(),
        ));
    }
    let mut reused_target_results = Vec::with_capacity(impact.exact_jobs.len());
    for job in &impact.exact_jobs {
        if job.definition_id != impact.target_definition_id
            || !impact
                .body_correspondence
                .iter()
                .any(|mapping| mapping.target_body_id == job.body_id)
        {
            return Err(ComponentReplacementCommitError::InvalidImpact(
                "component replacement exact job is outside the reviewed target correspondence"
                    .to_owned(),
            ));
        }
        let package = staged_results
            .get_body(&candidate, job.definition_id, job.body_id)
            .map_err(|error| ComponentReplacementCommitError::ExactPublication(error.to_string()))?
            .ok_or_else(|| {
                ComponentReplacementCommitError::ExactPublication(format!(
                    "target definition {} body {} has no current exact result",
                    job.definition_id.0, job.body_id.0
                ))
            })?;
        let request = ExactFeatureChainRequest::from_snapshot_for_body(
            &candidate,
            job.definition_id,
            job.body_id,
        )
        .map_err(|error| ComponentReplacementCommitError::InvalidImpact(error.to_string()))?;
        if package.producer_feature_id() != job.producer_feature_id
            || request.producer_feature_id() != job.producer_feature_id
            || request.canonical_input_digest != job.canonical_input_digest
            || package.result_key().result_fingerprint != job.last_valid_result_fingerprint
        {
            return Err(ComponentReplacementCommitError::Stale);
        }
        reused_target_results.push((job.body_id, package.result_key().result_fingerprint.clone()));
    }
    reused_target_results.sort_unstable();
    let selected_scene = candidate
        .scene_query()
        .into_iter()
        .find(|occurrence| occurrence.instance_path == impact.selected_instance_path)
        .ok_or_else(|| {
            ComponentReplacementCommitError::InvalidImpact(
                "selected occurrence render/pick projection disappeared".to_owned(),
            )
        })?;
    if selected_scene.occurrence_id != impact.selected_occurrence_id
        || selected_scene.definition_id != impact.target_definition_id
        || selected_scene.transform != impact.selected_transform
    {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "selected occurrence render/pick projection changed identity or transform".to_owned(),
        ));
    }

    if source.assembly_mates().count() != candidate.assembly_mates().count() {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement changed the assembly mate set".to_owned(),
        ));
    }
    for source_mate in source.assembly_mates() {
        let candidate_mate = candidate.assembly_mate(source_mate.id()).ok_or_else(|| {
            ComponentReplacementCommitError::InvalidImpact(format!(
                "assembly mate {} disappeared during component replacement",
                source_mate.id().0
            ))
        })?;
        if !expected_mate_ids.contains(&source_mate.id()) && candidate_mate != source_mate {
            return Err(ComponentReplacementCommitError::InvalidImpact(format!(
                "unrelated assembly mate {} changed during component replacement",
                source_mate.id().0
            )));
        }
        for endpoint in [candidate_mate.endpoint_a(), candidate_mate.endpoint_b()] {
            if endpoint.occurrence_id() == impact.selected_occurrence_id {
                match staged_results.resolve_reference(&candidate, endpoint.reference()) {
                    ExactReferenceResolution::Resolved { reference }
                        if reference.as_ref() == endpoint.reference() => {}
                    resolution => {
                        return Err(ComponentReplacementCommitError::ExactPublication(format!(
                            "assembly mate {} replacement reference is not uniquely current: {resolution:?}",
                            source_mate.id().0
                        )));
                    }
                }
            }
        }
    }
    let drawing_sheet_ids = impact
        .drawing_views
        .iter()
        .map(|view| view.sheet_id)
        .collect::<std::collections::BTreeSet<_>>();
    let drawings = drawing_sheet_ids
        .into_iter()
        .map(|sheet_id| {
            let sheet = candidate.drawing_sheet(sheet_id).ok_or_else(|| {
                ComponentReplacementCommitError::InvalidImpact(format!(
                    "drawing sheet {} disappeared during component replacement",
                    sheet_id.0
                ))
            })?;
            project_orthographic_drawing(&candidate, &staged_results, sheet).map_err(|error| {
                ComponentReplacementCommitError::ExactPublication(error.to_string())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if impact.exports.iter().any(|export| {
        export.eligibility != SharedChangeExportEligibility::CurrentExact
            || export.occurrence_paths.as_slice() != [impact.selected_instance_path.clone()]
    }) {
        return Err(ComponentReplacementCommitError::InvalidImpact(
            "component replacement export impact is outside the selected occurrence".to_owned(),
        ));
    }
    let export_bodies = impact
        .body_correspondence
        .iter()
        .map(|mapping| {
            staged_results
                .get_body(
                    &candidate,
                    impact.target_definition_id,
                    mapping.target_body_id,
                )
                .map_err(|error| {
                    ComponentReplacementCommitError::ExactPublication(error.to_string())
                })?
                .map(|package| (package.as_ref(), selected_scene.transform))
                .ok_or_else(|| {
                    ComponentReplacementCommitError::ExactPublication(format!(
                        "target body {} is unavailable for replacement export",
                        mapping.target_body_id.0
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    exact_model_stl_export(&candidate, &export_bodies)
        .map_err(|error| ComponentReplacementCommitError::ExactPublication(error.to_string()))?;

    let revision = document
        .commit_proposal(proposal)
        .map_err(|error: ProposalCommitError| {
            ComponentReplacementCommitError::Commit(error.to_string())
        })?;
    *exact_results = staged_results;

    Ok(ComponentReplacementCommitReceipt {
        revision_id: revision.id(),
        canonical_digest: revision.snapshot().canonical_digest(),
        selected_occurrence_id: impact.selected_occurrence_id,
        selected_instance_path: selected_scene.instance_path,
        selected_transform: selected_scene.transform,
        source_definition_id: impact.source_definition_id,
        target_definition_id: impact.target_definition_id,
        reused_target_results,
        rebound_mate_ids: rebound_mate_ids.into_iter().collect(),
        drawings,
        exports: impact.exports.clone(),
    })
}

fn replacement_target_reference(
    exact_results: &ExactResultRegistry,
    snapshot: &Snapshot,
    target_definition_id: DefinitionId,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
    mapping: &ComponentReplacementSubshapeCorrespondence,
) -> Result<BodySubshapeRef, ComponentReplacementImpactError> {
    let mut matches = Vec::new();
    for body in body_correspondence {
        let package = replacement_current_body_result(
            exact_results,
            snapshot,
            target_definition_id,
            body.target_body_id,
        )?;
        matches.extend(package.references().iter().filter(|reference| {
            reference.definition_id == target_definition_id
                && reference.profile_feature_id == mapping.target_profile_feature_id
                && reference.producer_feature_id == mapping.target_producer_feature_id
                && reference.lineage_digest == mapping.target_lineage_digest
                && reference.semantic_role == mapping.semantic_role
                && reference.source_element_id == mapping.source_element_id
                && reference.expected_type == mapping.expected_type
        }));
    }
    let [target] = matches.as_slice() else {
        return Err(ComponentReplacementImpactError::Incompatible(format!(
            "target subshape {} is not uniquely current",
            mapping.semantic_role
        )));
    };
    match exact_results.resolve_reference(snapshot, target) {
        ExactReferenceResolution::Resolved { reference } if reference.as_ref() == *target => {
            Ok((*target).clone())
        }
        ExactReferenceResolution::Ambiguous { .. } => {
            Err(ComponentReplacementImpactError::Ambiguous(
                target_definition_id,
                body_correspondence
                    .first()
                    .map_or(BodyId(0), |body| body.target_body_id),
            ))
        }
        ExactReferenceResolution::Lost => Err(ComponentReplacementImpactError::Incompatible(
            format!("target subshape {} was lost", mapping.semantic_role),
        )),
        ExactReferenceResolution::Quarantined { reason } => {
            Err(ComponentReplacementImpactError::Unsupported(format!(
                "target subshape {} is quarantined: {reason:?}",
                mapping.semantic_role
            )))
        }
        ExactReferenceResolution::Resolved { .. } => {
            Err(ComponentReplacementImpactError::Incompatible(format!(
                "target subshape {} resolved to different evidence",
                mapping.semantic_role
            )))
        }
    }
}

type ReplacementBodySignature = (String, Vec<(String, String, String)>);

fn replacement_body_signature(
    snapshot: &Snapshot,
    package: &ExactBodyPackage,
) -> Result<ReplacementBodySignature, ComponentReplacementImpactError> {
    let request = ExactFeatureChainRequest::from_snapshot_for_producer(
        snapshot,
        package.definition_id(),
        package.producer_feature_id(),
    )
    .map_err(|error| ComponentReplacementImpactError::Unsupported(error.to_string()))?;
    let mut references = package
        .references()
        .iter()
        .map(|reference| {
            (
                reference.semantic_role.clone(),
                reference.source_element_id.clone(),
                reference.expected_type.clone(),
            )
        })
        .collect::<Vec<_>>();
    references.sort();
    if references.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ComponentReplacementImpactError::Incompatible(
            "exact body contains duplicate semantic subshape identities".to_owned(),
        ));
    }
    Ok((request.evaluator().to_owned(), references))
}

fn replacement_current_body_result<'a>(
    exact_results: &'a ExactResultRegistry,
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    body_id: BodyId,
) -> Result<&'a Arc<ExactBodyPackage>, ComponentReplacementImpactError> {
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
                Err(ComponentReplacementImpactError::Stale)
            } else {
                Err(ComponentReplacementImpactError::Failed(
                    definition_id,
                    body_id,
                ))
            }
        }
        Err(ExactProductError::ConflictingBodyPublication {
            definition_id,
            body_id,
        }) => Err(ComponentReplacementImpactError::Ambiguous(
            definition_id,
            body_id,
        )),
        Err(error) => Err(ComponentReplacementImpactError::Unsupported(
            error.to_string(),
        )),
    }
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
        .map_err(|_| OccurrenceForkImpactError::Cyclic)?;

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
            let preview = prepare_body_parameter_edit(document, change, principal)
                .map_err(|error| OccurrenceForkImpactError::Unsupported(error.to_string()))?;
            (
                body_id,
                preview.affected_feature_ids,
                preview.proposal.batch().commands().to_vec(),
            )
        }
        SharedDefinitionChange::BodyHistoryMutation(change) => {
            let body_id = change.body_id;
            let preview = prepare_body_history_mutation(document, change, principal)
                .map_err(|error| OccurrenceForkImpactError::Unsupported(error.to_string()))?;
            (
                body_id,
                preview.affected_feature_ids,
                preview.proposal.batch().commands().to_vec(),
            )
        }
    };

    let source_definition = source.definition(source_definition_id).ok_or_else(|| {
        OccurrenceForkImpactError::Unsupported(format!(
            "definition {} was not found",
            source_definition_id.0
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
        next_feature_id = next_feature_id.checked_add(1).ok_or_else(|| {
            OccurrenceForkImpactError::Unsupported("feature identity space is exhausted".to_owned())
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
            .ok_or_else(|| {
                OccurrenceForkImpactError::Unsupported(format!(
                    "feature {} is outside the selected definition",
                    source_id.0
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
            match endpoint.health() {
                AssemblyReferenceHealth::Resolved => {}
                AssemblyReferenceHealth::Ambiguous { .. } => {
                    return Err(OccurrenceForkImpactError::Ambiguous(body_id));
                }
                AssemblyReferenceHealth::Lost => {
                    return Err(OccurrenceForkImpactError::Lost(mate.id()));
                }
                AssemblyReferenceHealth::Broken => {
                    return Err(OccurrenceForkImpactError::Unsupported(format!(
                        "assembly mate {} has a broken exact reference",
                        mate.id().0
                    )));
                }
            }
            match exact_results.resolve_reference(&source, endpoint.reference()) {
                ExactReferenceResolution::Resolved { .. } => {}
                ExactReferenceResolution::Ambiguous { .. } => {
                    return Err(OccurrenceForkImpactError::Ambiguous(body_id));
                }
                ExactReferenceResolution::Lost => {
                    return Err(OccurrenceForkImpactError::Lost(mate.id()));
                }
                ExactReferenceResolution::Quarantined { reason } => {
                    return Err(OccurrenceForkImpactError::Unsupported(format!(
                        "assembly mate {} exact reference is quarantined: {reason:?}",
                        mate.id().0
                    )));
                }
            }
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
                    "occurrence fork supports only existing exact edits and bounded history mutation"
                        .to_owned(),
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
        .map_err(|error| OccurrenceForkImpactError::Unsupported(error.to_string()))?;
    let candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| OccurrenceForkImpactError::Unsupported(error.to_string()))?;
    if candidate.definition(source_definition_id) != Some(source_definition) {
        return Err(OccurrenceForkImpactError::Unsupported(
            "occurrence fork candidate changed the source definition".to_owned(),
        ));
    }

    let exact_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&candidate, fork_definition_id, body_id)
            .map_err(|error| match error {
            ExactProductError::ConflictingBodyTerminals { .. }
            | ExactProductError::ConflictingBodyPublication { .. } => {
                OccurrenceForkImpactError::Ambiguous(body_id)
            }
            error => OccurrenceForkImpactError::Unsupported(error.to_string()),
        })?;
    let exact_jobs = vec![SharedChangeExactJob {
        definition_id: fork_definition_id,
        body_id,
        producer_feature_id: exact_request.producer_feature_id(),
        canonical_input_digest: exact_request.canonical_input_digest.clone(),
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
            .ok_or_else(|| {
                OccurrenceForkImpactError::Unsupported(format!(
                    "fork feature {} has no source lineage",
                    fork_id.0
                ))
            })
    };
    let fork_producer_feature_id = exact_request.producer_feature_id();
    let source_producer_feature_id = source_feature(fork_producer_feature_id)?;
    let mut subshape_lineage = exact_request
        .expected_face_roles()
        .iter()
        .copied()
        .map(|role| {
            let fork_profile_feature_id = exact_request
                .profile_feature_id_for_role(role)
                .ok_or_else(|| {
                    OccurrenceForkImpactError::Unsupported(format!(
                        "fork face role {} has no supported profile lineage",
                        role.semantic_role()
                    ))
                })?;
            let source_profile_feature_id = source_feature(fork_profile_feature_id)?;
            Ok(OccurrenceForkSubshapeLineage {
                source_definition_id,
                source_profile_feature_id,
                source_producer_feature_id,
                source_lineage_digest: canonical_reference_lineage_digest(
                    source.document_id(),
                    source_producer_feature_id,
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                ),
                fork_definition_id,
                fork_profile_feature_id,
                fork_producer_feature_id,
                fork_lineage_digest: canonical_reference_lineage_digest(
                    source.document_id(),
                    fork_producer_feature_id,
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                ),
                semantic_role: role.semantic_role().to_owned(),
                source_element_id: role.source_element_id().to_owned(),
                expected_type: role.expected_type().to_owned(),
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

    let affected_root = request.selected_occurrence_id;
    let mut drawing_views = Vec::new();
    for sheet in source.drawing_sheets() {
        if matches!(
            sheet.source(),
            DrawingSource::RigidAssembly { occurrence_ids }
                if occurrence_ids.contains(&affected_root)
        ) {
            for view in [
                OrthographicViewKind::Front,
                OrthographicViewKind::Top,
                OrthographicViewKind::Right,
            ] {
                drawing_views.push(SharedChangeDrawingViewImpact {
                    sheet_id: sheet.id(),
                    view,
                });
            }
        }
    }
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
        drawing_views,
        exports,
        proposal,
    })
}

fn next_fork_id(ids: impl Iterator<Item = u64>) -> Result<u64, OccurrenceForkImpactError> {
    ids.max().unwrap_or(0).checked_add(1).ok_or_else(|| {
        OccurrenceForkImpactError::Unsupported("identity space is exhausted".to_owned())
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
        SharedChangeImpactError::Cyclic => OccurrenceForkImpactError::Cyclic,
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
        .map_err(|_| SharedChangeImpactError::Cyclic)?;

    let (definition_id, body_id, affected_feature_ids, unchanged_body_ids, proposal) =
        match request.change {
            SharedDefinitionChange::ExactParameterEdit(change) => {
                let definition_id = change.definition_id;
                let body_id = change.body_id;
                let preview = prepare_body_parameter_edit(document, change, principal)
                    .map_err(|error| SharedChangeImpactError::Unsupported(error.to_string()))?;
                (
                    definition_id,
                    body_id,
                    preview.affected_feature_ids,
                    preview.unchanged_body_ids,
                    preview.proposal,
                )
            }
            SharedDefinitionChange::BodyHistoryMutation(change) => {
                let definition_id = change.definition_id;
                let body_id = change.body_id;
                let preview = prepare_body_history_mutation(document, change, principal)
                    .map_err(|error| SharedChangeImpactError::Unsupported(error.to_string()))?;
                (
                    definition_id,
                    body_id,
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

    let last_valid = current_body_result(exact_results, &source, definition_id, body_id)?;
    let candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| SharedChangeImpactError::Unsupported(error.to_string()))?;
    let exact_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&candidate, definition_id, body_id)
            .map_err(|error| map_exact_request_error(error, body_id))?;
    let exact_jobs = vec![SharedChangeExactJob {
        definition_id,
        body_id,
        producer_feature_id: exact_request.producer_feature_id(),
        canonical_input_digest: exact_request.canonical_input_digest,
        last_valid_result_fingerprint: last_valid.result_key().result_fingerprint,
    }];

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
            match endpoint.health() {
                AssemblyReferenceHealth::Resolved => {}
                AssemblyReferenceHealth::Ambiguous { .. } => {
                    return Err(SharedChangeImpactError::Ambiguous(body_id));
                }
                AssemblyReferenceHealth::Lost => {
                    return Err(SharedChangeImpactError::Lost(mate.id()));
                }
                AssemblyReferenceHealth::Broken => {
                    return Err(SharedChangeImpactError::Unsupported(format!(
                        "assembly mate {} has a broken exact reference",
                        mate.id().0
                    )));
                }
            }
            match exact_results.resolve_reference(&source, endpoint.reference()) {
                ExactReferenceResolution::Resolved { .. } => {}
                ExactReferenceResolution::Ambiguous { .. } => {
                    return Err(SharedChangeImpactError::Ambiguous(body_id));
                }
                ExactReferenceResolution::Lost => {
                    return Err(SharedChangeImpactError::Lost(mate.id()));
                }
                ExactReferenceResolution::Quarantined { reason } => {
                    return Err(SharedChangeImpactError::Unsupported(format!(
                        "assembly mate {} exact reference is quarantined: {reason:?}",
                        mate.id().0
                    )));
                }
            }
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
        };
        if affected {
            for view in [
                OrthographicViewKind::Front,
                OrthographicViewKind::Top,
                OrthographicViewKind::Right,
            ] {
                drawing_views.push(SharedChangeDrawingViewImpact {
                    sheet_id: sheet.id(),
                    view,
                });
            }
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
        affected_body_ids: vec![body_id],
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
        Err(error) => Err(SharedChangeImpactError::Unsupported(error.to_string())),
    }
}

fn map_exact_request_error(error: ExactProductError, body_id: BodyId) -> SharedChangeImpactError {
    match error {
        ExactProductError::ConflictingBodyTerminals { .. }
        | ExactProductError::ConflictingBodyPublication { .. } => {
            SharedChangeImpactError::Ambiguous(body_id)
        }
        error => SharedChangeImpactError::Unsupported(error.to_string()),
    }
}
