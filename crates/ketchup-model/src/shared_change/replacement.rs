//! Replacing the definition one root occurrence uses with another definition
//! whose bodies, features and subshapes correspond one to one: project the
//! impact for review, then commit exactly the reviewed impact.

use super::*;
use crate::assembly::AssemblyMateKind;
use crate::document::{Definition, FeatureDependencyGraph, Occurrence, SceneOccurrence};
use crate::exact_product::ExactReferenceQuarantineReason;

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
    pub collection_dependencies: Vec<OccurrenceCollectionDependencyImpact>,
    pub drawing_views: Vec<SharedChangeDrawingViewImpact>,
    pub exports: Vec<SharedChangeExportImpact>,
    pub proposal: Option<Proposal>,
}

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
    DependencyGraph(CanonicalError),
    /// The target definition does not correspond to the source definition.
    Incompatible(ReplacementMismatch),
    /// The replacement cannot carry a dependent part along.
    Unsupported(DependencyBlocker),
}

/// Where the target definition stops corresponding to the source definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplacementMismatch {
    BodyCount,
    FeatureCount,
    /// A source body has no single target body with the same exact signature.
    Body(BodyId),
    /// Corresponding bodies carry different numbers of subshapes.
    SubshapeCount {
        source: BodyId,
        target: BodyId,
    },
    /// An exact body names one semantic subshape twice.
    DuplicateSubshapeIdentity,
    Feature {
        feature: FeatureId,
        problem: FeatureMismatch,
    },
    Subshape {
        role: String,
        problem: SubshapeMismatch,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureMismatch {
    /// A dependency of the source feature has no target feature yet.
    Dependency(FeatureId),
    InputBody(BodyId),
    OutputBody(BodyId),
    NoUniqueTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubshapeMismatch {
    NoProfileFeature,
    NoProducerFeature,
    NoUniqueTarget,
    NotUniquelyCurrent,
    Lost,
    Quarantined(ExactReferenceQuarantineReason),
    ResolvedElsewhere,
    NoTypedAttachment,
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
            Self::DependencyGraph(error) => {
                write!(formatter, "feature dependency graph is invalid: {error}")
            }
            Self::Incompatible(mismatch) => mismatch.fmt(formatter),
            Self::Unsupported(blocker) => blocker.fmt(formatter),
        }
    }
}

impl fmt::Display for ReplacementMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BodyCount => {
                formatter.write_str("source and target definitions have different body counts")
            }
            Self::FeatureCount => {
                formatter.write_str("source and target definitions have different feature counts")
            }
            Self::Body(body) => write!(
                formatter,
                "source body {} has no unique compatible target body",
                body.0
            ),
            Self::SubshapeCount { source, target } => write!(
                formatter,
                "source body {} and target body {} have different subshape counts",
                source.0, target.0
            ),
            Self::DuplicateSubshapeIdentity => {
                formatter.write_str("exact body contains duplicate semantic subshape identities")
            }
            Self::Feature { feature, problem } => {
                write!(formatter, "source feature {} {problem}", feature.0)
            }
            Self::Subshape { role, problem } => write!(formatter, "subshape {role} {problem}"),
        }
    }
}

impl fmt::Display for FeatureMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dependency(feature) => {
                write!(formatter, "dependency {} has no target mapping", feature.0)
            }
            Self::InputBody(body) => {
                write!(formatter, "input body {} has no target mapping", body.0)
            }
            Self::OutputBody(body) => {
                write!(formatter, "output body {} has no target mapping", body.0)
            }
            Self::NoUniqueTarget => formatter.write_str("has no unique compatible target feature"),
        }
    }
}

impl fmt::Display for SubshapeMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProfileFeature => formatter.write_str("has no target profile feature"),
            Self::NoProducerFeature => formatter.write_str("has no target producer feature"),
            Self::NoUniqueTarget => formatter.write_str("has no unique target subshape"),
            Self::NotUniquelyCurrent => {
                formatter.write_str("is not uniquely current in the target")
            }
            Self::Lost => formatter.write_str("was lost in the target"),
            Self::Quarantined(reason) => {
                write!(formatter, "is quarantined in the target: {reason:?}")
            }
            Self::ResolvedElsewhere => formatter.write_str("resolved to different target evidence"),
            Self::NoTypedAttachment => {
                formatter.write_str("has no compatible typed assembly attachment in the target")
            }
        }
    }
}

impl std::error::Error for ComponentReplacementImpactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DependencyGraph(error) => Some(error),
            Self::Unsupported(blocker) => Some(blocker),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComponentReplacementCommitError {
    Stale,
    /// The reviewed impact no longer describes what the commit would do.
    InvalidImpact(ReviewMismatch),
    /// The staged exact results could not be published.
    ExactPublication(DependencyBlocker),
    Commit(String),
}

/// What in a reviewed component replacement no longer matches the document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewMismatch {
    NoProposal,
    NotReviewed,
    /// Projecting the impact again failed.
    Refresh(ComponentReplacementImpactError),
    /// Projecting the impact again gave a different correspondence.
    CorrespondenceChanged,
    UnrelatedCommand,
    /// The proposal does not hold exactly the reviewed repoint and rebinds.
    CommandsDiffer,
    SelectedMissing,
    SelectedDefinitionChanged,
    SelectedPlacementChanged,
    DefinitionEvidence,
    CollectionEvidence,
    DrawingEvidence,
    CandidateDigestChanged,
    /// An exact job targets a body outside the reviewed correspondence.
    ExactJobOutsideCorrespondence(BodyId),
    /// The render/pick projection of the selected occurrence disappeared.
    SceneMissing,
    /// The render/pick projection changed identity or transform.
    SceneChanged,
    Dependency(DependencyBlocker),
}

impl fmt::Display for ComponentReplacementCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("component replacement impact is stale"),
            Self::InvalidImpact(mismatch) => {
                write!(
                    formatter,
                    "component replacement impact is invalid: {mismatch}"
                )
            }
            Self::ExactPublication(blocker) => {
                write!(
                    formatter,
                    "component replacement exact publication failed: {blocker}"
                )
            }
            Self::Commit(reason) => {
                write!(formatter, "component replacement commit failed: {reason}")
            }
        }
    }
}

impl fmt::Display for ReviewMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProposal => formatter.write_str("it has no reviewed atomic proposal"),
            Self::NotReviewed => formatter.write_str("it was not reviewed"),
            Self::Refresh(error) => error.fmt(formatter),
            Self::CorrespondenceChanged => {
                formatter.write_str("it no longer matches the complete current correspondence")
            }
            Self::UnrelatedCommand => {
                formatter.write_str("its proposal contains an unrelated canonical command")
            }
            Self::CommandsDiffer => formatter.write_str(
                "its proposal does not match the reviewed repoint and dependency rebind",
            ),
            Self::SelectedMissing => {
                formatter.write_str("the selected occurrence disappeared from the source snapshot")
            }
            Self::SelectedDefinitionChanged => formatter
                .write_str("the selected occurrence no longer uses the reviewed source definition"),
            Self::SelectedPlacementChanged => formatter
                .write_str("the selected occurrence no longer has the reviewed world placement"),
            Self::DefinitionEvidence => {
                formatter.write_str("its definition-isolation evidence is incomplete")
            }
            Self::CollectionEvidence => {
                formatter.write_str("its collection dependency evidence is incomplete")
            }
            Self::DrawingEvidence => {
                formatter.write_str("its drawing dependency evidence is incomplete")
            }
            Self::CandidateDigestChanged => formatter.write_str("its candidate digest changed"),
            Self::ExactJobOutsideCorrespondence(body) => write!(
                formatter,
                "its exact job for body {} is outside the reviewed target correspondence",
                body.0
            ),
            Self::SceneMissing => {
                formatter.write_str("the selected occurrence render/pick projection disappeared")
            }
            Self::SceneChanged => formatter.write_str(
                "the selected occurrence render/pick projection changed identity or transform",
            ),
            Self::Dependency(blocker) => blocker.fmt(formatter),
        }
    }
}

impl std::error::Error for ComponentReplacementCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidImpact(ReviewMismatch::Refresh(error)) => Some(error),
            Self::InvalidImpact(ReviewMismatch::Dependency(blocker))
            | Self::ExactPublication(blocker) => Some(blocker),
            _ => None,
        }
    }
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
    let selection = select_replacement(&source, &request)?;
    let body_correspondence = correspond_bodies(exact_results, &source, &selection)?;
    let graph = source
        .feature_dependency_graph()
        .map_err(ComponentReplacementImpactError::DependencyGraph)?;
    let feature_correspondence =
        correspond_features(&source, &graph, &selection, &body_correspondence)?;
    let (subshape_correspondence, exact_jobs) = correspond_subshapes(
        exact_results,
        &source,
        &selection,
        &body_correspondence,
        &feature_correspondence,
    )?;
    let (mate_references, mate_rebind_commands) = rebind_replacement_mates(
        exact_results,
        &source,
        &selection,
        &body_correspondence,
        &subshape_correspondence,
    )?;
    let drawing_views = replacement_drawing_views(exact_results, &source, &selection)?;
    export_replacement(exact_results, &source, &selection, &body_correspondence)?;
    let exports = [
        SharedChangeExportFormat::Step,
        SharedChangeExportFormat::Stl,
    ]
    .into_iter()
    .map(|format| SharedChangeExportImpact {
        format,
        occurrence_paths: vec![selection.scene.instance_path.clone()],
        eligibility: SharedChangeExportEligibility::CurrentExact,
    })
    .collect();

    let unchanged_source_occurrences = unchanged_root_occurrences(
        &source,
        selection.source_definition_id,
        selection.occurrence_id,
    );
    let unchanged_target_occurrences = unchanged_root_occurrences(
        &source,
        selection.target_definition_id,
        selection.occurrence_id,
    );
    let unchanged_definition_ids = source
        .definitions()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();
    let mut commands = vec![CanonicalCommand::RepointOccurrence {
        id: selection.occurrence_id,
        definition_id: selection.target_definition_id,
    }];
    commands.extend(mate_rebind_commands);
    let protected_occurrence_ids = unchanged_source_occurrences
        .iter()
        .chain(&unchanged_target_occurrences)
        .map(|occurrence| occurrence.occurrence_id)
        .collect::<std::collections::BTreeSet<_>>();
    let solved_transform_ids = solve_replacement_mates(
        exact_results,
        &source,
        &selection,
        &mate_references,
        &protected_occurrence_ids,
        &mut commands,
    )?;
    let proposal = document
        .prepare_proposal_with_context(
            CommandBatch::new(commands),
            replacement_proposal_context(&selection, principal),
        )
        .map_err(|error| unsupported(DependencyBlocker::Proposal(error)))?;
    let candidate_digest = verify_replacement_candidate(
        document,
        exact_results,
        &source,
        &selection,
        &proposal,
        &body_correspondence,
        &solved_transform_ids,
    )?;

    Ok(ComponentReplacementImpactProjection {
        source_revision: source.revision_id(),
        source_digest: source.canonical_digest(),
        candidate_digest: Some(candidate_digest),
        selected_occurrence_id: selection.occurrence_id,
        selected_instance_path: selection.scene.instance_path.clone(),
        selected_transform: selection.scene.transform,
        source_definition_id: selection.source_definition_id,
        target_definition_id: selection.target_definition_id,
        body_correspondence,
        feature_correspondence,
        subshape_correspondence,
        unchanged_source_occurrences,
        unchanged_target_occurrences,
        unchanged_definition_ids,
        exact_jobs,
        mate_references,
        collection_dependencies: preserved_collection_dependencies(
            &source,
            selection.occurrence_id,
        ),
        drawing_views,
        exports,
        proposal: Some(proposal),
    })
}

/// The occurrence a replacement repoints and the two definitions it swaps.
struct ReplacementSelection<'a> {
    occurrence_id: OccurrenceId,
    occurrence: &'a Occurrence,
    scene: SceneOccurrence,
    source_definition_id: DefinitionId,
    source_definition: &'a Definition,
    target_definition_id: DefinitionId,
    target_definition: &'a Definition,
}

fn unsupported(blocker: DependencyBlocker) -> ComponentReplacementImpactError {
    ComponentReplacementImpactError::Unsupported(blocker)
}

fn incompatible(mismatch: ReplacementMismatch) -> ComponentReplacementImpactError {
    ComponentReplacementImpactError::Incompatible(mismatch)
}

fn subshape_mismatch(role: &str, problem: SubshapeMismatch) -> ComponentReplacementImpactError {
    incompatible(ReplacementMismatch::Subshape {
        role: role.to_owned(),
        problem,
    })
}

/// Checks the request against the current snapshot and finds the selected
/// root occurrence plus the source and target definitions.
fn select_replacement<'a>(
    source: &'a Snapshot,
    request: &ComponentReplacementImpactRequest,
) -> Result<ReplacementSelection<'a>, ComponentReplacementImpactError> {
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
        .map_err(ComponentReplacementImpactError::DependencyGraph)?;

    let [target_definition_id] = request.target_definition_ids.as_slice() else {
        return Err(ComponentReplacementImpactError::DuplicateTarget);
    };
    let occurrence_id = request.selected_occurrence_id;
    let occurrence = source.occurrence(occurrence_id).ok_or(
        ComponentReplacementImpactError::OccurrenceNotFound(occurrence_id),
    )?;
    let source_definition_id = occurrence.definition_id();
    if source_definition_id == *target_definition_id {
        return Err(ComponentReplacementImpactError::SelfReplacement(
            source_definition_id,
        ));
    }
    let scene = source
        .scene_query()
        .into_iter()
        .find(|scene| scene.occurrence_id == occurrence_id && scene.instance_path.is_root())
        .ok_or(ComponentReplacementImpactError::OccurrenceNotFound(
            occurrence_id,
        ))?;
    if !scene.visible {
        return Err(ComponentReplacementImpactError::Hidden(occurrence_id));
    }

    let source_definition = source.definition(source_definition_id).ok_or(unsupported(
        DependencyBlocker::DefinitionMissing(source_definition_id),
    ))?;
    let target_definition = source.definition(*target_definition_id).ok_or(
        ComponentReplacementImpactError::TargetDefinitionNotFound(*target_definition_id),
    )?;
    if [source_definition, target_definition]
        .iter()
        .any(|definition| {
            !definition.local_occurrence_ids().is_empty()
                || !definition.local_group_ids().is_empty()
        })
    {
        return Err(unsupported(DependencyBlocker::NestedInstances));
    }
    Ok(ReplacementSelection {
        occurrence_id,
        occurrence,
        scene,
        source_definition_id,
        source_definition,
        target_definition_id: *target_definition_id,
        target_definition,
    })
}

fn visible_body_ids(
    definition: &Definition,
) -> Result<Vec<BodyId>, ComponentReplacementImpactError> {
    definition
        .bodies()
        .map(|body| {
            if !body.visible() || body.consumed_by().is_some() {
                Err(unsupported(DependencyBlocker::BodyHiddenOrConsumed {
                    definition: definition.id(),
                    body: body.id(),
                }))
            } else {
                Ok(body.id())
            }
        })
        .collect()
}

/// Pairs every source body with the one unused target body whose exact
/// signature matches.
fn correspond_bodies(
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
) -> Result<Vec<ComponentReplacementBodyCorrespondence>, ComponentReplacementImpactError> {
    let source_body_ids = visible_body_ids(selection.source_definition)?;
    let target_body_ids = visible_body_ids(selection.target_definition)?;
    if source_body_ids.len() != target_body_ids.len() {
        return Err(incompatible(ReplacementMismatch::BodyCount));
    }

    let mut body_correspondence = Vec::with_capacity(source_body_ids.len());
    let mut used_target_bodies = std::collections::BTreeSet::new();
    for source_body_id in source_body_ids {
        let source_package = replacement_current_body_result(
            exact_results,
            source,
            selection.source_definition_id,
            source_body_id,
        )?;
        let source_signature = replacement_body_signature(source, source_package)?;
        let mut matches = Vec::new();
        for target_body_id in &target_body_ids {
            if used_target_bodies.contains(target_body_id) {
                continue;
            }
            let target_package = replacement_current_body_result(
                exact_results,
                source,
                selection.target_definition_id,
                *target_body_id,
            )?;
            if replacement_body_signature(source, target_package)? == source_signature {
                matches.push(*target_body_id);
            }
        }
        let [target_body_id] = matches.as_slice() else {
            return Err(incompatible(ReplacementMismatch::Body(source_body_id)));
        };
        used_target_bodies.insert(*target_body_id);
        body_correspondence.push(ComponentReplacementBodyCorrespondence {
            source_body_id,
            target_body_id: *target_body_id,
        });
    }
    body_correspondence.sort_unstable();
    Ok(body_correspondence)
}

fn definition_feature_ids(
    source: &Snapshot,
    graph: &FeatureDependencyGraph,
    definition_id: DefinitionId,
) -> Vec<FeatureId> {
    graph
        .topological_order()
        .iter()
        .copied()
        .filter(|feature_id| {
            source
                .feature(*feature_id)
                .is_some_and(|feature| feature.definition_id() == definition_id)
        })
        .collect()
}

fn mapped_body(
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
    body_id: BodyId,
) -> Option<BodyId> {
    body_correspondence
        .iter()
        .find_map(|mapping| (mapping.source_body_id == body_id).then_some(mapping.target_body_id))
}

fn mapped_feature(
    feature_correspondence: &[ComponentReplacementFeatureCorrespondence],
    feature_id: FeatureId,
) -> Option<FeatureId> {
    feature_correspondence.iter().find_map(|mapping| {
        (mapping.source_feature_id == feature_id).then_some(mapping.target_feature_id)
    })
}

/// Pairs every source feature, in dependency order, with the one unused
/// target feature of the same kind, suppression, dependencies and bodies.
fn correspond_features(
    source: &Snapshot,
    graph: &FeatureDependencyGraph,
    selection: &ReplacementSelection<'_>,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
) -> Result<Vec<ComponentReplacementFeatureCorrespondence>, ComponentReplacementImpactError> {
    let source_feature_ids = definition_feature_ids(source, graph, selection.source_definition_id);
    let target_feature_ids = definition_feature_ids(source, graph, selection.target_definition_id);
    if source_feature_ids.len() != target_feature_ids.len() {
        return Err(incompatible(ReplacementMismatch::FeatureCount));
    }

    let mut feature_correspondence = Vec::with_capacity(source_feature_ids.len());
    let mut used_target_features = std::collections::BTreeSet::new();
    for source_feature_id in source_feature_ids {
        let mismatch = |problem| {
            incompatible(ReplacementMismatch::Feature {
                feature: source_feature_id,
                problem,
            })
        };
        let source_feature = source.feature(source_feature_id).ok_or(unsupported(
            DependencyBlocker::FeatureMissing(source_feature_id),
        ))?;
        let source_dependencies = graph.dependencies(source_feature_id).ok_or(unsupported(
            DependencyBlocker::FeatureDependenciesMissing(source_feature_id),
        ))?;
        let mapped_dependencies = source_dependencies
            .iter()
            .map(|dependency| {
                mapped_feature(&feature_correspondence, *dependency)
                    .ok_or_else(|| mismatch(FeatureMismatch::Dependency(*dependency)))
            })
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        let source_ownership = selection
            .source_definition
            .feature_body_ownership(source_feature_id);
        let mapped_inputs = source_ownership
            .map(|ownership| {
                ownership
                    .input_body_ids()
                    .iter()
                    .map(|body_id| {
                        mapped_body(body_correspondence, *body_id)
                            .ok_or_else(|| mismatch(FeatureMismatch::InputBody(*body_id)))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let mapped_output = source_ownership
            .and_then(|ownership| ownership.output_body_id())
            .map(|body_id| {
                mapped_body(body_correspondence, body_id)
                    .ok_or_else(|| mismatch(FeatureMismatch::OutputBody(body_id)))
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
                    selection
                        .target_definition
                        .feature_body_ownership(*target_feature_id),
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
            return Err(mismatch(FeatureMismatch::NoUniqueTarget));
        };
        used_target_features.insert(*target_feature_id);
        feature_correspondence.push(ComponentReplacementFeatureCorrespondence {
            source_feature_id,
            target_feature_id: *target_feature_id,
        });
    }
    feature_correspondence.sort_unstable();
    Ok(feature_correspondence)
}

/// Pairs every semantic subshape of every source body with its one target
/// subshape and lists the exact job each target body would reuse.
fn correspond_subshapes(
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
    feature_correspondence: &[ComponentReplacementFeatureCorrespondence],
) -> Result<
    (
        Vec<ComponentReplacementSubshapeCorrespondence>,
        Vec<SharedChangeExactJob>,
    ),
    ComponentReplacementImpactError,
> {
    let mut subshape_correspondence = Vec::new();
    let mut exact_jobs = Vec::with_capacity(body_correspondence.len());
    for body_mapping in body_correspondence {
        let source_package = replacement_current_body_result(
            exact_results,
            source,
            selection.source_definition_id,
            body_mapping.source_body_id,
        )?;
        let target_package = replacement_current_body_result(
            exact_results,
            source,
            selection.target_definition_id,
            body_mapping.target_body_id,
        )?;
        for source_reference in source_package.references() {
            let role = &source_reference.semantic_role;
            let mapped_profile =
                mapped_feature(feature_correspondence, source_reference.profile_feature_id)
                    .ok_or_else(|| subshape_mismatch(role, SubshapeMismatch::NoProfileFeature))?;
            let mapped_producer =
                mapped_feature(feature_correspondence, source_reference.producer_feature_id)
                    .ok_or_else(|| subshape_mismatch(role, SubshapeMismatch::NoProducerFeature))?;
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
                return Err(subshape_mismatch(role, SubshapeMismatch::NoUniqueTarget));
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
            return Err(incompatible(ReplacementMismatch::SubshapeCount {
                source: body_mapping.source_body_id,
                target: body_mapping.target_body_id,
            }));
        }
        let exact_graph = producer_exact_graph(
            source,
            selection.target_definition_id,
            target_package.producer_feature_id(),
        )
        .map_err(|error| unsupported(DependencyBlocker::Exact(error)))?;
        exact_jobs.push(SharedChangeExactJob {
            definition_id: selection.target_definition_id,
            body_id: body_mapping.target_body_id,
            producer_feature_id: target_package.producer_feature_id(),
            canonical_input_digest: exact_graph.canonical_input_digest,
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
    Ok((subshape_correspondence, exact_jobs))
}

/// Moves every mate endpoint on the selected occurrence to the corresponding
/// target subshape. Returns the reviewed mate references and rebind commands.
fn rebind_replacement_mates(
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
    subshape_correspondence: &[ComponentReplacementSubshapeCorrespondence],
) -> Result<
    (
        Vec<ComponentReplacementMateReferenceImpact>,
        Vec<CanonicalCommand>,
    ),
    ComponentReplacementImpactError,
> {
    let mut mate_references = Vec::new();
    let mut mate_rebind_commands = Vec::new();
    for mate in source.assembly_mates() {
        let mut rebound_endpoints = Vec::with_capacity(2);
        let mut changed = false;
        for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
            if endpoint.occurrence_id() != selection.occurrence_id {
                rebound_endpoints.push(endpoint.clone());
                continue;
            }
            let mapping = replacement_mate_mapping(
                exact_results,
                source,
                selection,
                mate,
                endpoint,
                subshape_correspondence,
            )?;
            mate_references.push(ComponentReplacementMateReferenceImpact {
                mate_id: mate.id(),
                occurrence_id: selection.occurrence_id,
                source_lineage_digest: mapping.source_lineage_digest.clone(),
                target_lineage_digest: mapping.target_lineage_digest.clone(),
            });
            let target_reference = replacement_target_reference(
                exact_results,
                source,
                selection.target_definition_id,
                body_correspondence,
                mapping,
            )?;
            let rebound_endpoint = match mate.kind() {
                AssemblyMateKind::CoincidentPlanar { .. } => exact_results
                    .planar_face_attachment(source, &target_reference)
                    .cloned()
                    .map(|attachment| {
                        AssemblyMateEndpoint::resolved_planar_face(
                            selection.occurrence_id,
                            attachment,
                        )
                    }),
                AssemblyMateKind::ConcentricAxial { .. } => exact_results
                    .axial_attachment(source, &target_reference)
                    .cloned()
                    .map(|attachment| {
                        AssemblyMateEndpoint::resolved_axial(selection.occurrence_id, attachment)
                    }),
                _ => None,
            }
            .ok_or_else(|| {
                subshape_mismatch(&mapping.semantic_role, SubshapeMismatch::NoTypedAttachment)
            })?;
            rebound_endpoints.push(rebound_endpoint);
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
    Ok((mate_references, mate_rebind_commands))
}

/// Checks that one mate endpoint on the selected occurrence is a healthy
/// planar or axial reference into the source definition and finds the
/// subshape correspondence it moves along.
fn replacement_mate_mapping<'c>(
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
    mate: &AssemblyMate,
    endpoint: &AssemblyMateEndpoint,
    subshape_correspondence: &'c [ComponentReplacementSubshapeCorrespondence],
) -> Result<&'c ComponentReplacementSubshapeCorrespondence, ComponentReplacementImpactError> {
    let mate_blocker = |problem| {
        unsupported(DependencyBlocker::Mate {
            mate: mate.id(),
            problem,
        })
    };
    if !matches!(
        mate.kind(),
        AssemblyMateKind::CoincidentPlanar { .. } | AssemblyMateKind::ConcentricAxial { .. }
    ) {
        return Err(mate_blocker(MateProblem::NotPlanarOrAxial));
    }
    if endpoint.reference().definition_id != selection.source_definition_id {
        return Err(mate_blocker(MateProblem::NotSourceDefinition));
    }
    current_mate_endpoint(exact_results, source, endpoint).map_err(|refusal| match refusal {
        EndpointRefusal::Ambiguous => ComponentReplacementImpactError::Ambiguous(
            selection.source_definition_id,
            selection.source_definition.active_body_id(),
        ),
        EndpointRefusal::Lost => ComponentReplacementImpactError::Lost(mate.id()),
        EndpointRefusal::Blocked(problem) => mate_blocker(problem),
    })?;
    subshape_correspondence
        .iter()
        .find(|mapping| {
            mapping.source_profile_feature_id == endpoint.reference().profile_feature_id
                && mapping.source_producer_feature_id == endpoint.reference().producer_feature_id
                && mapping.source_lineage_digest == endpoint.reference().lineage_digest
        })
        .ok_or_else(|| mate_blocker(MateProblem::NoTargetSubshape))
}

/// Lists the views of every drawing sheet that shows the selected occurrence,
/// after checking each sheet still projects from current evidence.
fn replacement_drawing_views(
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
) -> Result<Vec<SharedChangeDrawingViewImpact>, ComponentReplacementImpactError> {
    let mut drawing_views = Vec::new();
    for sheet in selected_sheets(source, selection.occurrence_id) {
        let drawing =
            project_orthographic_drawing(source, exact_results, sheet).map_err(|error| {
                unsupported(DependencyBlocker::Sheet {
                    sheet: sheet.id(),
                    problem: SheetProblem::Projection(error),
                })
            })?;
        if !drawing.is_current(source) {
            return Err(ComponentReplacementImpactError::Stale);
        }
        drawing_views.extend(sheet_views(sheet.id()));
    }
    drawing_views.sort_unstable();
    Ok(drawing_views)
}

/// Exports the target bodies at the selected placement, proving the
/// replaced occurrence stays exportable.
fn export_replacement(
    exact_results: &ExactResultRegistry,
    snapshot: &Snapshot,
    selection: &ReplacementSelection<'_>,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
) -> Result<(), ComponentReplacementImpactError> {
    let bodies = body_correspondence
        .iter()
        .map(|mapping| {
            replacement_current_body_result(
                exact_results,
                snapshot,
                selection.target_definition_id,
                mapping.target_body_id,
            )
            .map(|package| (package.as_ref(), selection.scene.transform))
        })
        .collect::<Result<Vec<_>, _>>()?;
    exact_model_stl_export(snapshot, &bodies)
        .map_err(|error| unsupported(DependencyBlocker::Exact(error)))?;
    Ok(())
}

fn unchanged_root_occurrences(
    source: &Snapshot,
    definition_id: DefinitionId,
    selected_occurrence_id: OccurrenceId,
) -> Vec<SharedChangeOccurrenceImpact> {
    let mut occurrences = source
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.instance_path.is_root()
                && occurrence.definition_id == definition_id
                && occurrence.occurrence_id != selected_occurrence_id
        })
        .map(|occurrence| SharedChangeOccurrenceImpact {
            occurrence_id: occurrence.occurrence_id,
            instance_path: occurrence.instance_path,
            visible: occurrence.visible,
        })
        .collect::<Vec<_>>();
    occurrences.sort_by(|left, right| left.instance_path.cmp(&right.instance_path));
    occurrences
}

/// Solves the rigid mates touched by the rebinds and appends the solve to
/// `commands`. Returns the occurrences the solve moves; it may not move the
/// selected occurrence nor any `protected` one.
fn solve_replacement_mates(
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
    mate_references: &[ComponentReplacementMateReferenceImpact],
    protected: &std::collections::BTreeSet<OccurrenceId>,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<std::collections::BTreeSet<OccurrenceId>, ComponentReplacementImpactError> {
    let direct_mate_ids = mate_references
        .iter()
        .map(|reference| reference.mate_id)
        .collect::<std::collections::BTreeSet<_>>();
    if direct_mate_ids.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    let rebound_candidate = source
        .preview_batch(&CommandBatch::new(commands.clone()))
        .map_err(|error| unsupported(DependencyBlocker::Staging(error)))?;
    let rebound_results = ExactResultRegistry::carried_forward(&rebound_candidate, exact_results);
    let affected_mate_ids = dependent_mate_component(&rebound_candidate, &direct_mate_ids);
    let recomputed = recompute_rigid_assembly_mates_from_snapshot(
        &rebound_candidate,
        &rebound_results,
        AssemblySolverPolicy::default(),
        &affected_mate_ids,
    )
    .map_err(|error| unsupported(DependencyBlocker::AssemblyRecompute(error)))?;
    let solve = recomputed
        .solve()
        .ok_or(unsupported(DependencyBlocker::NoSolveResult))?;
    if recomputed.status() != AssemblyRecomputeStatus::Solved
        || solve.status() != AssemblySolveStatus::FullyConstrained
        || !solve.conflicting_mate_ids().is_empty()
        || !solve.maximum_residual().is_finite()
    {
        return Err(unsupported(DependencyBlocker::SolveNotFullyConstrained {
            recompute: recomputed.status(),
            solve: solve.status(),
        }));
    }
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
    for (occurrence_id, _) in &transforms {
        if *occurrence_id == selection.occurrence_id {
            return Err(unsupported(DependencyBlocker::SolveMovesSelected));
        }
        if protected.contains(occurrence_id) {
            return Err(unsupported(DependencyBlocker::SolveMovesSibling));
        }
    }
    let solved = transforms
        .iter()
        .map(|(occurrence_id, _)| *occurrence_id)
        .collect();
    if !transforms.is_empty() {
        commands.push(CanonicalCommand::ApplyAssemblySolve {
            source_revision: source.revision_id(),
            source_digest: source.canonical_digest(),
            transforms,
            instance_transforms: Vec::new(),
        });
    }
    Ok(solved)
}

fn replacement_proposal_context(
    selection: &ReplacementSelection<'_>,
    principal: ProposalPrincipal,
) -> ProposalContext {
    ProposalContext {
        principal,
        goal: ProposalGoal::CanonicalPreview,
        assumptions: vec![
            ProposalAssumption::TargetExists(AuthoritativeDependency::Occurrence(
                selection.occurrence_id,
            )),
            ProposalAssumption::TargetExists(AuthoritativeDependency::Definition(
                selection.source_definition_id,
            )),
            ProposalAssumption::TargetExists(AuthoritativeDependency::Definition(
                selection.target_definition_id,
            )),
        ],
        risk: ProposalRisk::Standard,
        confirmation: ProposalConfirmation::ReviewRequired,
        requested_budget: ProposalBudget::HOST_MAX,
    }
}

/// Whether `candidate` keeps the identity, world placement, collections and
/// drawing sources of the occurrence `selected` while repointing it to
/// `target_definition_id`.
fn candidate_keeps_selected(
    source: &Snapshot,
    candidate: &Snapshot,
    selected: &Occurrence,
    target_definition_id: DefinitionId,
    world_transform: Transform,
) -> bool {
    let occurrence_id = selected.id();
    candidate
        .occurrence(occurrence_id)
        .is_some_and(|candidate_selected| {
            candidate_selected.definition_id() == target_definition_id
                && candidate_selected.name() == selected.name()
                && candidate_selected.transform() == selected.transform()
                && candidate_selected.parent() == selected.parent()
                && candidate_selected.tags() == selected.tags()
                && candidate_selected.visible() == selected.visible()
        })
        && candidate.occurrence_is_grounded(occurrence_id)
            == source.occurrence_is_grounded(occurrence_id)
        && candidate
            .world_transform_for_occurrence(occurrence_id)
            .is_some_and(|transform| transforms_nearly_equal(transform, world_transform))
        && source.collections().eq(candidate.collections())
        && source.drawing_sheets().eq(candidate.drawing_sheets())
}

/// Finds what `candidate` changed beyond the selected occurrence, the
/// occurrences the solve moved, and nothing in any definition.
fn candidate_drift(
    source: &Snapshot,
    candidate: &Snapshot,
    selected_occurrence_id: OccurrenceId,
    solved_transform_ids: &std::collections::BTreeSet<OccurrenceId>,
) -> Option<CandidateDrift> {
    source
        .occurrences()
        .filter(|occurrence| occurrence.id() != selected_occurrence_id)
        .find(|occurrence| {
            !solved_transform_ids.contains(&occurrence.id())
                && candidate.occurrence(occurrence.id()) != Some(*occurrence)
        })
        .map(|occurrence| CandidateDrift::Occurrence(occurrence.id()))
        .or_else(|| {
            source
                .definitions()
                .find(|definition| candidate.definition(definition.id()) != Some(*definition))
                .map(|definition| CandidateDrift::Definition(definition.id()))
        })
}

/// Stages the proposal and proves the candidate keeps everything the
/// replacement must keep, still projects its drawings and still exports the
/// replaced occurrence. Returns the candidate digest.
fn verify_replacement_candidate(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    selection: &ReplacementSelection<'_>,
    proposal: &Proposal,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
    solved_transform_ids: &std::collections::BTreeSet<OccurrenceId>,
) -> Result<String, ComponentReplacementImpactError> {
    let candidate = document
        .preview_batch(proposal.batch())
        .map_err(|error| unsupported(DependencyBlocker::Staging(error)))?;
    let candidate_results = ExactResultRegistry::carried_forward(&candidate, exact_results);
    if candidate.occurrence(selection.occurrence_id).is_none() {
        return Err(ComponentReplacementImpactError::OccurrenceNotFound(
            selection.occurrence_id,
        ));
    }
    if !candidate_keeps_selected(
        source,
        &candidate,
        selection.occurrence,
        selection.target_definition_id,
        selection.scene.transform,
    ) {
        return Err(unsupported(DependencyBlocker::CandidateChanged(
            CandidateDrift::SelectedOccurrence,
        )));
    }
    if let Some(drift) = candidate_drift(
        source,
        &candidate,
        selection.occurrence_id,
        solved_transform_ids,
    ) {
        return Err(unsupported(DependencyBlocker::CandidateChanged(drift)));
    }
    for sheet in selected_sheets(source, selection.occurrence_id) {
        project_orthographic_drawing(&candidate, &candidate_results, sheet).map_err(|error| {
            unsupported(DependencyBlocker::Sheet {
                sheet: sheet.id(),
                problem: SheetProblem::Projection(error),
            })
        })?;
    }
    export_replacement(
        &candidate_results,
        &candidate,
        selection,
        body_correspondence,
    )?;
    Ok(candidate.canonical_digest())
}

pub fn commit_component_replacement(
    document: &mut DocumentStore,
    exact_results: &mut ExactResultRegistry,
    impact: &ComponentReplacementImpactProjection,
) -> Result<ComponentReplacementCommitReceipt, ComponentReplacementCommitError> {
    let source = document.current();
    let proposal = reviewed_replacement_proposal(document, exact_results, &source, impact)?;
    let rebound_mate_ids = check_replacement_commands(&source, impact, proposal)?;
    let selected = check_replacement_evidence(&source, impact)?;

    let candidate = document.preview_batch(proposal.batch()).map_err(|error| {
        invalid(ReviewMismatch::Dependency(DependencyBlocker::Staging(
            error,
        )))
    })?;
    if impact.candidate_digest.as_deref() != Some(candidate.canonical_digest().as_str()) {
        return Err(invalid(ReviewMismatch::CandidateDigestChanged));
    }
    if candidate
        .occurrence(impact.selected_occurrence_id)
        .is_none()
        || !candidate_keeps_selected(
            &source,
            &candidate,
            selected,
            impact.target_definition_id,
            impact.selected_transform,
        )
    {
        return Err(invalid(ReviewMismatch::Dependency(
            DependencyBlocker::CandidateChanged(CandidateDrift::SelectedOccurrence),
        )));
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
    if let Some(drift) = candidate_drift(
        &source,
        &candidate,
        impact.selected_occurrence_id,
        &solved_transform_ids,
    ) {
        return Err(invalid(ReviewMismatch::Dependency(
            DependencyBlocker::CandidateChanged(drift),
        )));
    }

    let staged_results = ExactResultRegistry::carried_forward(&candidate, exact_results);
    if staged_results.values().count() != exact_results.values().count() {
        return Err(ComponentReplacementCommitError::ExactPublication(
            DependencyBlocker::ResultsNotCarried,
        ));
    }
    let reused_target_results = reused_replacement_results(&staged_results, &candidate, impact)?;
    let selected_scene = candidate
        .scene_query()
        .into_iter()
        .find(|occurrence| occurrence.instance_path == impact.selected_instance_path)
        .ok_or(invalid(ReviewMismatch::SceneMissing))?;
    if selected_scene.occurrence_id != impact.selected_occurrence_id
        || selected_scene.definition_id != impact.target_definition_id
        || selected_scene.transform != impact.selected_transform
    {
        return Err(invalid(ReviewMismatch::SceneChanged));
    }
    check_replaced_mates(
        &source,
        &candidate,
        &staged_results,
        impact,
        &rebound_mate_ids,
    )?;
    let drawings = publish_replacement_outputs(&candidate, &staged_results, impact)?;

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

fn invalid(mismatch: ReviewMismatch) -> ComponentReplacementCommitError {
    ComponentReplacementCommitError::InvalidImpact(mismatch)
}

fn publication(blocker: DependencyBlocker) -> ComponentReplacementCommitError {
    ComponentReplacementCommitError::ExactPublication(blocker)
}

/// Returns the reviewed proposal of `impact` after proving the impact is
/// current: the same revision, reviewed, and projected again identically.
fn reviewed_replacement_proposal<'i>(
    document: &DocumentStore,
    exact_results: &ExactResultRegistry,
    source: &Snapshot,
    impact: &'i ComponentReplacementImpactProjection,
) -> Result<&'i Proposal, ComponentReplacementCommitError> {
    if source.revision_id() != impact.source_revision
        || source.canonical_digest() != impact.source_digest
    {
        return Err(ComponentReplacementCommitError::Stale);
    }
    let proposal = impact
        .proposal
        .as_ref()
        .ok_or(invalid(ReviewMismatch::NoProposal))?;
    if proposal.provenance_revision() != impact.source_revision
        || proposal.provenance_digest() != impact.source_digest
    {
        return Err(ComponentReplacementCommitError::Stale);
    }
    if !matches!(
        proposal.confirmation(),
        ProposalConfirmation::ReviewRequired
    ) {
        return Err(invalid(ReviewMismatch::NotReviewed));
    }
    let refreshed = project_component_replacement_impact_for_principal(
        document,
        exact_results,
        ComponentReplacementImpactRequest::new(
            source,
            impact.selected_occurrence_id,
            impact.target_definition_id,
        ),
        proposal.principal(),
    )
    .map_err(|error| match error {
        ComponentReplacementImpactError::Stale => ComponentReplacementCommitError::Stale,
        error => invalid(ReviewMismatch::Refresh(error)),
    })?;
    if refreshed != *impact {
        return Err(invalid(ReviewMismatch::CorrespondenceChanged));
    }
    Ok(proposal)
}

/// Proves the proposal holds exactly one repoint of the selected occurrence,
/// the reviewed mate rebinds and at most one solve that leaves the protected
/// occurrences alone. Returns the rebound mates.
fn check_replacement_commands(
    source: &Snapshot,
    impact: &ComponentReplacementImpactProjection,
    proposal: &Proposal,
) -> Result<std::collections::BTreeSet<AssemblyMateId>, ComponentReplacementCommitError> {
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
                check_reviewed_rebind(source, impact, rebound)?;
                rebound_mate_ids.insert(rebound.id());
            }
            CanonicalCommand::ApplyAssemblySolve { transforms, .. }
                if transforms.iter().all(|(occurrence_id, _)| {
                    !protected_occurrence_ids.contains(occurrence_id)
                }) =>
            {
                solve_count += 1;
            }
            _ => return Err(invalid(ReviewMismatch::UnrelatedCommand)),
        }
    }
    if repoint_count != 1 || solve_count > 1 || rebound_mate_ids != expected_mate_ids {
        return Err(invalid(ReviewMismatch::CommandsDiffer));
    }
    Ok(rebound_mate_ids)
}

/// Proves one rebind keeps the mate kind and its other endpoint and moves the
/// selected endpoint to a resolved reviewed target subshape.
fn check_reviewed_rebind(
    source: &Snapshot,
    impact: &ComponentReplacementImpactProjection,
    rebound: &AssemblyMate,
) -> Result<(), ComponentReplacementCommitError> {
    let mate_mismatch = |problem| {
        invalid(ReviewMismatch::Dependency(DependencyBlocker::Mate {
            mate: rebound.id(),
            problem,
        }))
    };
    let source_mate = source
        .assembly_mate(rebound.id())
        .ok_or_else(|| mate_mismatch(MateProblem::Introduced))?;
    if rebound.kind() != source_mate.kind() {
        return Err(mate_mismatch(MateProblem::KindChanged));
    }
    for (before, after) in [source_mate.endpoint_a(), source_mate.endpoint_b()]
        .into_iter()
        .zip([rebound.endpoint_a(), rebound.endpoint_b()])
    {
        let selected = before.occurrence_id() == impact.selected_occurrence_id;
        if before.occurrence_id() != after.occurrence_id()
            || (!selected && before != after)
            || (selected
                && (after.health() != AssemblyReferenceHealth::Resolved
                    || after.reference().definition_id != impact.target_definition_id
                    || !impact.mate_references.iter().any(|expected| {
                        expected.mate_id == rebound.id()
                            && expected.target_lineage_digest == after.reference().lineage_digest
                    })))
        {
            return Err(mate_mismatch(MateProblem::OtherEndpointChanged));
        }
    }
    Ok(())
}

/// Proves the selected occurrence, the definitions, the collections and the
/// drawings still match what the impact reviewed. Returns the selected
/// occurrence.
fn check_replacement_evidence<'s>(
    source: &'s Snapshot,
    impact: &ComponentReplacementImpactProjection,
) -> Result<&'s Occurrence, ComponentReplacementCommitError> {
    let selected = source
        .occurrence(impact.selected_occurrence_id)
        .ok_or(invalid(ReviewMismatch::SelectedMissing))?;
    if selected.definition_id() != impact.source_definition_id {
        return Err(invalid(ReviewMismatch::SelectedDefinitionChanged));
    }
    if source
        .world_transform_for_occurrence(impact.selected_occurrence_id)
        .is_none_or(|transform| !transforms_nearly_equal(transform, impact.selected_transform))
    {
        return Err(invalid(ReviewMismatch::SelectedPlacementChanged));
    }
    if !source
        .definitions()
        .map(|definition| definition.id())
        .eq(impact.unchanged_definition_ids.iter().copied())
    {
        return Err(invalid(ReviewMismatch::DefinitionEvidence));
    }
    if preserved_collection_dependencies(source, impact.selected_occurrence_id)
        != impact.collection_dependencies
    {
        return Err(invalid(ReviewMismatch::CollectionEvidence));
    }
    let mut expected_drawing_views = selected_sheets(source, impact.selected_occurrence_id)
        .flat_map(|sheet| sheet_views(sheet.id()))
        .collect::<Vec<_>>();
    expected_drawing_views.sort_unstable();
    if expected_drawing_views != impact.drawing_views {
        return Err(invalid(ReviewMismatch::DrawingEvidence));
    }
    Ok(selected)
}

/// Proves every reviewed exact job of the target definition still has its
/// last-valid result in `staged_results`. Returns the reused fingerprints.
fn reused_replacement_results(
    staged_results: &ExactResultRegistry,
    candidate: &Snapshot,
    impact: &ComponentReplacementImpactProjection,
) -> Result<Vec<(BodyId, String)>, ComponentReplacementCommitError> {
    let mut reused_target_results = Vec::with_capacity(impact.exact_jobs.len());
    for job in &impact.exact_jobs {
        if job.definition_id != impact.target_definition_id
            || !impact
                .body_correspondence
                .iter()
                .any(|mapping| mapping.target_body_id == job.body_id)
        {
            return Err(invalid(ReviewMismatch::ExactJobOutsideCorrespondence(
                job.body_id,
            )));
        }
        let package = staged_target_body(staged_results, candidate, impact, job.body_id)?;
        let graph =
            body_exact_graph(candidate, job.definition_id, job.body_id).map_err(|error| {
                invalid(ReviewMismatch::Dependency(DependencyBlocker::Exact(error)))
            })?;
        if package.producer_feature_id() != job.producer_feature_id
            || FeatureId(graph.producer_feature_id) != job.producer_feature_id
            || graph.canonical_input_digest != job.canonical_input_digest
            || package.result_key().result_fingerprint != job.last_valid_result_fingerprint
        {
            return Err(ComponentReplacementCommitError::Stale);
        }
        reused_target_results.push((job.body_id, package.result_key().result_fingerprint.clone()));
    }
    reused_target_results.sort_unstable();
    Ok(reused_target_results)
}

fn staged_target_body<'r>(
    staged_results: &'r ExactResultRegistry,
    candidate: &Snapshot,
    impact: &ComponentReplacementImpactProjection,
    body_id: BodyId,
) -> Result<&'r Arc<ExactBodyPackage>, ComponentReplacementCommitError> {
    staged_results
        .get_body(candidate, impact.target_definition_id, body_id)
        .map_err(|error| publication(DependencyBlocker::Exact(error)))?
        .ok_or_else(|| {
            publication(DependencyBlocker::Export {
                path: impact.selected_instance_path.clone(),
                problem: ExportProblem::NoExactBody,
            })
        })
}

/// Proves the candidate keeps the mate set, changes only the reviewed mates
/// and resolves every selected endpoint uniquely in the staged results.
fn check_replaced_mates(
    source: &Snapshot,
    candidate: &Snapshot,
    staged_results: &ExactResultRegistry,
    impact: &ComponentReplacementImpactProjection,
    rebound_mate_ids: &std::collections::BTreeSet<AssemblyMateId>,
) -> Result<(), ComponentReplacementCommitError> {
    if source.assembly_mates().count() != candidate.assembly_mates().count() {
        return Err(invalid(ReviewMismatch::Dependency(
            DependencyBlocker::MateSetChanged,
        )));
    }
    for source_mate in source.assembly_mates() {
        let mate = source_mate.id();
        let candidate_mate = candidate.assembly_mate(mate).ok_or_else(|| {
            invalid(ReviewMismatch::Dependency(DependencyBlocker::Mate {
                mate,
                problem: MateProblem::Missing,
            }))
        })?;
        if !rebound_mate_ids.contains(&mate) && candidate_mate != source_mate {
            return Err(invalid(ReviewMismatch::Dependency(
                DependencyBlocker::Mate {
                    mate,
                    problem: MateProblem::UnrelatedChanged,
                },
            )));
        }
        for endpoint in [candidate_mate.endpoint_a(), candidate_mate.endpoint_b()] {
            if endpoint.occurrence_id() != impact.selected_occurrence_id {
                continue;
            }
            match staged_results.resolve_reference(candidate, endpoint.reference()) {
                ExactReferenceResolution::Resolved { reference }
                    if reference.as_ref() == endpoint.reference() => {}
                resolution => {
                    return Err(publication(DependencyBlocker::Mate {
                        mate,
                        problem: MateProblem::NotUniquelyCurrent(resolution),
                    }));
                }
            }
        }
    }
    Ok(())
}

/// Projects the reviewed drawing sheets and exports the replaced occurrence
/// from the staged results. Returns the drawings.
fn publish_replacement_outputs(
    candidate: &Snapshot,
    staged_results: &ExactResultRegistry,
    impact: &ComponentReplacementImpactProjection,
) -> Result<Vec<OrthographicDrawing>, ComponentReplacementCommitError> {
    let drawing_sheet_ids = impact
        .drawing_views
        .iter()
        .map(|view| view.sheet_id)
        .collect::<std::collections::BTreeSet<_>>();
    let drawings = drawing_sheet_ids
        .into_iter()
        .map(|sheet_id| {
            let sheet = candidate.drawing_sheet(sheet_id).ok_or_else(|| {
                invalid(ReviewMismatch::Dependency(DependencyBlocker::Sheet {
                    sheet: sheet_id,
                    problem: SheetProblem::Missing,
                }))
            })?;
            project_orthographic_drawing(candidate, staged_results, sheet).map_err(|error| {
                publication(DependencyBlocker::Sheet {
                    sheet: sheet_id,
                    problem: SheetProblem::Projection(error),
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if impact.exports.iter().any(|export| {
        export.eligibility != SharedChangeExportEligibility::CurrentExact
            || export.occurrence_paths.as_slice() != [impact.selected_instance_path.clone()]
    }) {
        return Err(invalid(ReviewMismatch::Dependency(
            DependencyBlocker::ExportOutsideSelection,
        )));
    }
    let export_bodies = impact
        .body_correspondence
        .iter()
        .map(|mapping| {
            staged_target_body(staged_results, candidate, impact, mapping.target_body_id)
                .map(|package| (package.as_ref(), impact.selected_transform))
        })
        .collect::<Result<Vec<_>, _>>()?;
    exact_model_stl_export(candidate, &export_bodies)
        .map_err(|error| publication(DependencyBlocker::Exact(error)))?;
    Ok(drawings)
}

fn replacement_target_reference(
    exact_results: &ExactResultRegistry,
    snapshot: &Snapshot,
    target_definition_id: DefinitionId,
    body_correspondence: &[ComponentReplacementBodyCorrespondence],
    mapping: &ComponentReplacementSubshapeCorrespondence,
) -> Result<BodySubshapeRef, ComponentReplacementImpactError> {
    let mismatch = |problem| subshape_mismatch(&mapping.semantic_role, problem);
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
        return Err(mismatch(SubshapeMismatch::NotUniquelyCurrent));
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
        ExactReferenceResolution::Lost => Err(mismatch(SubshapeMismatch::Lost)),
        ExactReferenceResolution::Quarantined { reason } => {
            Err(mismatch(SubshapeMismatch::Quarantined(reason)))
        }
        ExactReferenceResolution::Resolved { .. } => {
            Err(mismatch(SubshapeMismatch::ResolvedElsewhere))
        }
    }
}

type ReplacementBodySignature = (String, Vec<(String, String, String)>);

fn replacement_body_signature(
    snapshot: &Snapshot,
    package: &ExactBodyPackage,
) -> Result<ReplacementBodySignature, ComponentReplacementImpactError> {
    producer_exact_graph(
        snapshot,
        package.definition_id(),
        package.producer_feature_id(),
    )
    .map_err(|error| unsupported(DependencyBlocker::Exact(error)))?;
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
        return Err(incompatible(ReplacementMismatch::DuplicateSubshapeIdentity));
    }
    Ok((EXACT_BREP_GRAPH_EVALUATOR_V1.to_owned(), references))
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
        Err(error) => Err(unsupported(DependencyBlocker::Exact(error))),
    }
}
