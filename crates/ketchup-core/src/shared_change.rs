#![forbid(unsafe_code)]

use crate::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyRecomputeStatus,
    AssemblyReferenceHealth, AssemblySolverPolicy, recompute_rigid_assembly_mates_from_snapshot,
};
use crate::document::DocumentStore;
use crate::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, FeatureId, InstancePath, OccurrenceId,
    Proposal, ProposalCommitError, ProposalConfirmation, ProposalContext, ProposalPrincipal,
    Snapshot, Transform,
};
use crate::drawing::{
    DrawingSheetId, DrawingSource, OrthographicDrawing, OrthographicViewKind,
    project_orthographic_drawing,
};
use crate::exact_product::{
    ExactBodyPackage, ExactFeatureChainRequest, ExactProductError, ExactReferenceResolution,
    ExactResultRegistry, exact_model_stl_export,
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
