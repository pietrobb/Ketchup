use super::*;
use ketchup_core::feature_history::{
    BodyHistoryMutation, BodyHistoryMutationRequest, BodyProfileTranslationRequest,
    ExactParameterEdit, ExactParameterEditTarget, FeatureHistoryProjection, FeatureHistoryQuery,
    FeatureHistoryState, RollbackPreviewRequest, prepare_body_history_mutation,
    prepare_body_parameter_edit, prepare_body_profile_translation, project_feature_history,
};
use ketchup_core::shared_change::{
    ComponentReplacementImpactProjection, ComponentReplacementImpactRequest,
    OccurrenceForkChangeRequest, OccurrenceForkImpactProjection, SharedChangeExportEligibility,
    SharedChangeExportFormat, SharedChangeImpactError, SharedChangeImpactProjection,
    SharedDefinitionChangeRequest, commit_component_replacement, commit_occurrence_fork_change,
    commit_shared_definition_change, project_component_replacement_impact,
    project_occurrence_fork_impact, project_shared_change_impact,
};
use ketchup_core::sketch::SketchConstraintKind;

#[derive(Clone, Debug, Eq, PartialEq)]
enum FeatureHistoryPreviewKind {
    ExactEdit,
    ProfileTranslation,
    Suppress { boundary: FeatureId },
    Resume,
    ReplaceComponent,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FeatureHistoryChangeScope {
    #[default]
    Shared,
    MakeUnique,
    ReplaceComponent,
}

#[derive(Clone, Debug, PartialEq)]
enum FeatureHistoryExecutionPlan {
    Local(Proposal),
    Shared(SharedChangeImpactProjection),
    Fork(OccurrenceForkImpactProjection),
    Replacement(ComponentReplacementImpactProjection),
}

impl FeatureHistoryExecutionPlan {
    fn shared_impact(&self) -> Option<&SharedChangeImpactProjection> {
        match self {
            Self::Shared(impact) => Some(impact),
            Self::Local(_) | Self::Fork(_) | Self::Replacement(_) => None,
        }
    }

    fn fork_impact(&self) -> Option<&OccurrenceForkImpactProjection> {
        match self {
            Self::Fork(impact) => Some(impact),
            Self::Local(_) | Self::Shared(_) | Self::Replacement(_) => None,
        }
    }

    fn replacement_impact(&self) -> Option<&ComponentReplacementImpactProjection> {
        match self {
            Self::Replacement(impact) => Some(impact),
            Self::Local(_) | Self::Shared(_) | Self::Fork(_) => None,
        }
    }

    const fn is_propagated(&self) -> bool {
        !matches!(self, Self::Local(_))
    }

    fn proposal(&self) -> Option<&Proposal> {
        match self {
            Self::Local(proposal) => Some(proposal),
            Self::Shared(impact) => Some(&impact.proposal),
            Self::Fork(impact) => Some(&impact.proposal),
            Self::Replacement(impact) => impact.proposal.as_ref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum FeatureHistoryPreviewSource {
    ExactEdit {
        source_revision: u64,
        source_digest: String,
        request: ketchup_core::feature_history::BodyParameterEditRequest,
        fork: Option<(OccurrenceId, String)>,
    },
    ProfileTranslation {
        source_revision: u64,
        source_digest: String,
        request: BodyProfileTranslationRequest,
    },
    Mutation {
        source_revision: u64,
        source_digest: String,
        request: BodyHistoryMutationRequest,
        fork: Option<(OccurrenceId, String)>,
    },
    Replacement(ComponentReplacementImpactRequest),
}

#[derive(Clone, Debug, PartialEq)]
struct FeatureHistoryUiPreview {
    source: FeatureHistoryPreviewSource,
    execution: FeatureHistoryExecutionPlan,
    kind: FeatureHistoryPreviewKind,
    action: String,
    affected_feature_ids: Vec<FeatureId>,
    suppressed_feature_ids: Vec<FeatureId>,
}

#[derive(Clone, Debug)]
struct ParameterChoice {
    target: ExactParameterEditTarget,
    label: String,
    value_mm: f64,
}

#[derive(Default)]
pub(super) struct FeatureHistoryUiState {
    definition: Option<DefinitionId>,
    selected_body: Option<ketchup_core::document::BodyId>,
    selected_feature: Option<FeatureId>,
    selected_parameter: Option<ExactParameterEditTarget>,
    parameter_source: Option<(u64, ExactParameterEditTarget)>,
    value_input: String,
    move_input: String,
    change_scope: FeatureHistoryChangeScope,
    replacement_target: Option<DefinitionId>,
    preview: Option<FeatureHistoryUiPreview>,
}

enum FeatureHistoryUiAction {
    SelectDefinition(DefinitionId),
    SelectBody(ketchup_core::document::BodyId),
    SelectFeature(FeatureId),
    SelectParameter(ExactParameterEditTarget),
    SelectReplacementTarget(DefinitionId),
    PreviewEdit,
    PreviewMove,
    PreviewSuppress,
    PreviewResume,
    PreviewReplacement,
    Confirm,
    Cancel,
}

fn ids(values: &[FeatureId]) -> String {
    if values.is_empty() {
        "—".to_owned()
    } else {
        values
            .iter()
            .map(|id| id.0.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn body_ids(values: &[ketchup_core::document::BodyId]) -> String {
    if values.is_empty() {
        "—".to_owned()
    } else {
        values
            .iter()
            .map(|id| id.0.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl KetchupApp {
    fn feature_history_error(&mut self, reason: impl ToString) {
        self.digest = self.catalog.format(
            "feature-history-error",
            &BTreeMap::from([("reason", reason.to_string())]),
        );
    }

    fn feature_parameter_choices(
        &self,
        snapshot: &Snapshot,
        feature_id: FeatureId,
    ) -> Vec<ParameterChoice> {
        let Some(feature) = snapshot.feature(feature_id) else {
            return Vec::new();
        };
        match feature.kind() {
            FeatureKind::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Offset { distance, .. } => vec![ParameterChoice {
                    target: ExactParameterEditTarget::FeatureDimension(feature_id),
                    label: self.catalog.text("feature-history-parameter-offset"),
                    value_mm: distance.millimetres(),
                }],
                WorkplaneSupport::Principal(_) | WorkplaneSupport::PlanarFace { .. } => Vec::new(),
            },
            FeatureKind::Sketch(spec) => spec
                .constraints
                .iter()
                .filter_map(|constraint| {
                    let value = match &constraint.kind {
                        SketchConstraintKind::Distance { value, .. }
                        | SketchConstraintKind::Radius { value, .. } => value,
                        SketchConstraintKind::Horizontal { .. }
                        | SketchConstraintKind::Vertical { .. }
                        | SketchConstraintKind::Coincident { .. }
                        | SketchConstraintKind::FixedPoint { .. } => return None,
                    };
                    Some(ParameterChoice {
                        target: ExactParameterEditTarget::SketchConstraintDimension {
                            sketch_id: feature_id,
                            constraint_id: constraint.id,
                        },
                        label: self.catalog.format(
                            "feature-history-parameter-constraint",
                            &BTreeMap::from([("id", constraint.id.0.to_string())]),
                        ),
                        value_mm: value.millimetres(),
                    })
                })
                .collect(),
            FeatureKind::Extrusion { height, .. } => vec![ParameterChoice {
                target: ExactParameterEditTarget::FeatureDimension(feature_id),
                label: self.catalog.text("feature-history-parameter-extent"),
                value_mm: height.millimetres(),
            }],
            FeatureKind::Pad(spec) => spec
                .extent
                .blind_distance()
                .map(|distance| ParameterChoice {
                    target: ExactParameterEditTarget::FeatureDimension(feature_id),
                    label: self.catalog.text("feature-history-parameter-extent"),
                    value_mm: distance.millimetres(),
                })
                .into_iter()
                .collect(),
            FeatureKind::SketchPocket(spec) => spec
                .extent
                .blind_distance()
                .map(|distance| ParameterChoice {
                    target: ExactParameterEditTarget::FeatureDimension(feature_id),
                    label: self.catalog.text("feature-history-parameter-depth"),
                    value_mm: distance.millimetres(),
                })
                .into_iter()
                .collect(),
            FeatureKind::Pocket { depth, .. } => vec![ParameterChoice {
                target: ExactParameterEditTarget::FeatureDimension(feature_id),
                label: self.catalog.text("feature-history-parameter-depth"),
                value_mm: depth.millimetres(),
            }],
            _ => Vec::new(),
        }
    }

    fn selected_feature_subshape_reference(
        &self,
        definition_id: DefinitionId,
        feature_id: FeatureId,
    ) -> Option<ketchup_core::exact_product::BodySubshapeRef> {
        let selection = self.selection.primary.as_ref()?;
        if selection.definition_id != definition_id {
            return None;
        }
        let role = match selection.element {
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            } => ExactFaceRole::Top,
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Minimum,
            } => ExactFaceRole::Bottom,
            ElementId::Face {
                axis: Axis::X,
                side: Side::Maximum,
            } => ExactFaceRole::East,
            _ => return None,
        };
        let mut references = self
            .exact_results
            .values()
            .filter(|package| {
                package.definition_id() == definition_id
                    && package.producer_feature_id() == feature_id
            })
            .filter_map(|package| package.reference(role).cloned());
        let reference = references.next()?;
        references.next().is_none().then_some(reference)
    }

    fn selected_fork_occurrence(&self, definition_id: DefinitionId) -> Option<OccurrenceId> {
        let snapshot = self.document.current();
        let mut selected = self.selected_occurrence_ids().into_iter().filter(|id| {
            snapshot
                .occurrence(*id)
                .is_some_and(|occurrence| occurrence.definition_id() == definition_id)
        });
        let occurrence_id = selected.next()?;
        selected.next().is_none().then_some(occurrence_id)
    }

    fn fork_definition_name(&self, snapshot: &Snapshot, definition_id: DefinitionId) -> String {
        let name = snapshot.definition(definition_id).map_or_else(
            || definition_id.0.to_string(),
            |definition| definition.name().to_owned(),
        );
        self.catalog
            .format("model-unique-name", &BTreeMap::from([("name", name)]))
    }

    fn reconcile_feature_history(
        &mut self,
        snapshot: &Snapshot,
    ) -> Option<FeatureHistoryProjection> {
        let definitions = snapshot
            .definitions()
            .map(|definition| definition.id())
            .collect::<Vec<_>>();
        if self
            .feature_history
            .definition
            .is_none_or(|id| !definitions.contains(&id))
        {
            self.feature_history.definition = definitions.first().copied();
            self.feature_history.selected_body = None;
            self.feature_history.selected_feature = None;
            self.feature_history.selected_parameter = None;
            self.feature_history.parameter_source = None;
        }
        let definition_id = self.feature_history.definition?;
        let base = match project_feature_history(
            snapshot,
            &self.exact_results,
            definition_id,
            &FeatureHistoryQuery::default(),
        ) {
            Ok(projection) => projection,
            Err(error) => {
                self.feature_history_error(error);
                return None;
            }
        };
        if self
            .feature_history
            .selected_body
            .is_none_or(|id| !base.bodies.iter().any(|body| body.body_id == id))
        {
            self.feature_history.selected_body = base
                .bodies
                .iter()
                .find(|body| body.active)
                .or_else(|| base.bodies.first())
                .map(|body| body.body_id);
            self.feature_history.selected_feature = None;
            self.feature_history.selected_parameter = None;
            self.feature_history.parameter_source = None;
        }
        let selected_body = self.feature_history.selected_body?;
        let body = base
            .bodies
            .iter()
            .find(|body| body.body_id == selected_body)?;
        if self
            .feature_history
            .selected_feature
            .is_none_or(|id| !body.features.iter().any(|entry| entry.feature_id == id))
        {
            self.feature_history.selected_feature =
                body.features.last().map(|entry| entry.feature_id);
            self.feature_history.selected_parameter = None;
            self.feature_history.parameter_source = None;
        }
        let query = FeatureHistoryQuery {
            selected_feature_id: self.feature_history.selected_feature,
            selected_subshape: self
                .feature_history
                .selected_feature
                .and_then(|feature_id| {
                    self.selected_feature_subshape_reference(definition_id, feature_id)
                }),
            rollback_preview: self.feature_history.preview.as_ref().and_then(
                |preview| match preview.kind {
                    FeatureHistoryPreviewKind::Suppress { boundary } => {
                        Some(RollbackPreviewRequest {
                            body_id: selected_body,
                            first_suppressed_feature_id: boundary,
                        })
                    }
                    FeatureHistoryPreviewKind::ExactEdit
                    | FeatureHistoryPreviewKind::ProfileTranslation
                    | FeatureHistoryPreviewKind::Resume
                    | FeatureHistoryPreviewKind::ReplaceComponent => None,
                },
            ),
        };
        match project_feature_history(snapshot, &self.exact_results, definition_id, &query) {
            Ok(projection) => Some(projection),
            Err(error) => {
                self.feature_history_error(error);
                None
            }
        }
    }

    fn derive_feature_history_preview(
        &self,
        source: &FeatureHistoryPreviewSource,
    ) -> Result<FeatureHistoryUiPreview, String> {
        let snapshot = self.document.current();
        let (execution, kind, affected_feature_ids, suppressed_feature_ids, action) = match source {
            FeatureHistoryPreviewSource::ExactEdit {
                source_revision,
                source_digest,
                request,
                fork,
            } => {
                if snapshot.revision_id() != *source_revision
                    || snapshot.canonical_digest() != source_digest.as_str()
                {
                    return Err("feature history preview source is stale".to_owned());
                }
                let (execution, affected) = if let Some((occurrence_id, fork_name)) = fork {
                    let impact = project_occurrence_fork_impact(
                        &self.document,
                        &self.exact_results,
                        OccurrenceForkChangeRequest::exact_parameter_edit(
                            &snapshot,
                            *occurrence_id,
                            fork_name.clone(),
                            request.clone(),
                        ),
                        ProposalPrincipal::ManualClient,
                    )
                    .map_err(|error| error.to_string())?;
                    let affected = impact.affected_fork_feature_ids.clone();
                    (FeatureHistoryExecutionPlan::Fork(impact), affected)
                } else {
                    match project_shared_change_impact(
                        &self.document,
                        &self.exact_results,
                        SharedDefinitionChangeRequest::exact_parameter_edit(
                            &snapshot,
                            request.clone(),
                        ),
                        ProposalPrincipal::ManualClient,
                    ) {
                        Ok(impact) => {
                            let affected = impact.affected_feature_ids.clone();
                            (FeatureHistoryExecutionPlan::Shared(impact), affected)
                        }
                        Err(SharedChangeImpactError::DefinitionNotReused(_)) => {
                            let preview = prepare_body_parameter_edit(
                                &self.document,
                                request.clone(),
                                ProposalPrincipal::ManualClient,
                            )
                            .map_err(|error| error.to_string())?;
                            (
                                FeatureHistoryExecutionPlan::Local(preview.proposal),
                                preview.affected_feature_ids,
                            )
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                };
                (
                    execution,
                    FeatureHistoryPreviewKind::ExactEdit,
                    affected,
                    Vec::new(),
                    self.catalog.text("feature-history-action-edit"),
                )
            }
            FeatureHistoryPreviewSource::ProfileTranslation {
                source_revision,
                source_digest,
                request,
            } => {
                if snapshot.revision_id() != *source_revision
                    || snapshot.canonical_digest() != source_digest.as_str()
                {
                    return Err("feature history preview source is stale".to_owned());
                }
                let preview = prepare_body_profile_translation(
                    &self.document,
                    request.clone(),
                    ProposalPrincipal::ManualClient,
                )
                .map_err(|error| error.to_string())?;
                (
                    FeatureHistoryExecutionPlan::Local(preview.proposal),
                    FeatureHistoryPreviewKind::ProfileTranslation,
                    preview.affected_feature_ids,
                    Vec::new(),
                    self.catalog.text("feature-history-action-move-profile"),
                )
            }
            FeatureHistoryPreviewSource::Mutation {
                source_revision,
                source_digest,
                request,
                fork,
            } => {
                if snapshot.revision_id() != *source_revision
                    || snapshot.canonical_digest() != source_digest.as_str()
                {
                    return Err("feature history preview source is stale".to_owned());
                }
                let kind = match request.mutation {
                    BodyHistoryMutation::SuppressFrom(boundary) => {
                        FeatureHistoryPreviewKind::Suppress { boundary }
                    }
                    BodyHistoryMutation::Resume => FeatureHistoryPreviewKind::Resume,
                };
                let (execution, affected, suppressed) = if let Some((occurrence_id, fork_name)) =
                    fork
                {
                    let impact = project_occurrence_fork_impact(
                        &self.document,
                        &self.exact_results,
                        OccurrenceForkChangeRequest::body_history_mutation(
                            &snapshot,
                            *occurrence_id,
                            fork_name.clone(),
                            *request,
                        ),
                        ProposalPrincipal::ManualClient,
                    )
                    .map_err(|error| error.to_string())?;
                    let affected = impact.affected_fork_feature_ids.clone();
                    let suppressed = match kind {
                        FeatureHistoryPreviewKind::Suppress { .. } => affected.clone(),
                        FeatureHistoryPreviewKind::Resume => Vec::new(),
                        FeatureHistoryPreviewKind::ExactEdit
                        | FeatureHistoryPreviewKind::ProfileTranslation
                        | FeatureHistoryPreviewKind::ReplaceComponent => unreachable!(),
                    };
                    (
                        FeatureHistoryExecutionPlan::Fork(impact),
                        affected,
                        suppressed,
                    )
                } else {
                    match project_shared_change_impact(
                        &self.document,
                        &self.exact_results,
                        SharedDefinitionChangeRequest::body_history_mutation(&snapshot, *request),
                        ProposalPrincipal::ManualClient,
                    ) {
                        Ok(impact) => {
                            let affected = impact.affected_feature_ids.clone();
                            let suppressed = match kind {
                                FeatureHistoryPreviewKind::Suppress { boundary } => affected
                                    .iter()
                                    .copied()
                                    .filter(|feature_id| *feature_id >= boundary)
                                    .collect(),
                                FeatureHistoryPreviewKind::Resume => Vec::new(),
                                FeatureHistoryPreviewKind::ExactEdit
                                | FeatureHistoryPreviewKind::ProfileTranslation
                                | FeatureHistoryPreviewKind::ReplaceComponent => unreachable!(),
                            };
                            (
                                FeatureHistoryExecutionPlan::Shared(impact),
                                affected,
                                suppressed,
                            )
                        }
                        Err(SharedChangeImpactError::DefinitionNotReused(_)) => {
                            let preview = prepare_body_history_mutation(
                                &self.document,
                                *request,
                                ProposalPrincipal::ManualClient,
                            )
                            .map_err(|error| error.to_string())?;
                            (
                                FeatureHistoryExecutionPlan::Local(preview.proposal),
                                preview.affected_feature_ids,
                                preview.suppressed_feature_ids,
                            )
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                };
                let action = self.catalog.text(match kind {
                    FeatureHistoryPreviewKind::Suppress { .. } => "feature-history-action-suppress",
                    FeatureHistoryPreviewKind::Resume => "feature-history-action-resume",
                    FeatureHistoryPreviewKind::ExactEdit
                    | FeatureHistoryPreviewKind::ProfileTranslation
                    | FeatureHistoryPreviewKind::ReplaceComponent => unreachable!(),
                });
                (execution, kind, affected, suppressed, action)
            }
            FeatureHistoryPreviewSource::Replacement(request) => {
                let impact = project_component_replacement_impact(
                    &self.document,
                    &self.exact_results,
                    request.clone(),
                )
                .map_err(|error| error.to_string())?;
                if impact.proposal.is_none() {
                    return Err("component replacement has no reviewed proposal".to_owned());
                }
                let affected = impact
                    .feature_correspondence
                    .iter()
                    .map(|mapping| mapping.source_feature_id)
                    .collect();
                (
                    FeatureHistoryExecutionPlan::Replacement(impact),
                    FeatureHistoryPreviewKind::ReplaceComponent,
                    affected,
                    Vec::new(),
                    self.catalog
                        .text("feature-history-action-replace-component"),
                )
            }
        };
        let preview = FeatureHistoryUiPreview {
            source: source.clone(),
            execution,
            kind,
            action,
            affected_feature_ids,
            suppressed_feature_ids,
        };
        let proposal = preview
            .execution
            .proposal()
            .ok_or_else(|| "feature history preview has no reviewed proposal".to_owned())?;
        self.document
            .preview_batch(proposal.batch())
            .map_err(|error| error.to_string())?;
        Ok(preview)
    }

    fn prepare_feature_history_edit(&mut self) -> bool {
        let (Some(definition_id), Some(body_id), Some(target)) = (
            self.feature_history.definition,
            self.feature_history.selected_body,
            self.feature_history.selected_parameter,
        ) else {
            self.feature_history_error(self.catalog.text("feature-history-error-no-parameter"));
            return false;
        };
        let Some(dimension) = parse_dimension(&self.feature_history.value_input) else {
            self.feature_history_error(self.catalog.text("feature-history-error-invalid-value"));
            return false;
        };
        let request = ketchup_core::feature_history::BodyParameterEditRequest {
            definition_id,
            body_id,
            edits: vec![ExactParameterEdit { target, dimension }],
        };
        let snapshot = self.document.current();
        let fork = match self.feature_history.change_scope {
            FeatureHistoryChangeScope::Shared => None,
            FeatureHistoryChangeScope::MakeUnique => {
                let Some(occurrence_id) = self.selected_fork_occurrence(definition_id) else {
                    self.feature_history_error(
                        self.catalog.text("feature-history-error-select-occurrence"),
                    );
                    return false;
                };
                Some((
                    occurrence_id,
                    self.fork_definition_name(&snapshot, definition_id),
                ))
            }
            FeatureHistoryChangeScope::ReplaceComponent => return false,
        };
        let source = FeatureHistoryPreviewSource::ExactEdit {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest().to_owned(),
            request,
            fork,
        };
        let preview = match self.derive_feature_history_preview(&source) {
            Ok(preview) => preview,
            Err(error) => {
                self.feature_history_error(error);
                return false;
            }
        };
        let action = preview.action.clone();
        self.feature_history.preview = Some(preview);
        self.digest = self.catalog.format(
            "feature-history-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    pub(crate) fn assistant_profile_translation_target(
        &self,
    ) -> Option<(DefinitionId, BodyId, FeatureId, String)> {
        let definition_id = self.feature_history.definition?;
        let body_id = self.feature_history.selected_body?;
        let profile_id = self.feature_history.selected_feature?;
        let feature = self.document.current().feature(profile_id)?.clone();
        (feature.definition_id() == definition_id).then(|| {
            (
                definition_id,
                body_id,
                profile_id,
                feature.name().to_owned(),
            )
        })
    }

    pub(crate) fn assistant_parameter_edit_target(
        &self,
    ) -> Option<(DefinitionId, BodyId, ExactParameterEditTarget, String, f64)> {
        let definition_id = self.feature_history.definition?;
        let body_id = self.feature_history.selected_body?;
        let target = self.feature_history.selected_parameter?;
        let feature_id = match target {
            ExactParameterEditTarget::FeatureDimension(feature_id)
            | ExactParameterEditTarget::SketchConstraintDimension {
                sketch_id: feature_id,
                ..
            } => feature_id,
        };
        let choice = self
            .feature_parameter_choices(&self.document.current(), feature_id)
            .into_iter()
            .find(|choice| choice.target == target)?;
        Some((
            definition_id,
            body_id,
            target,
            choice.label,
            choice.value_mm,
        ))
    }

    fn prepare_feature_profile_translation(&mut self) -> bool {
        let (Some(definition_id), Some(body_id), Some(profile_id)) = (
            self.feature_history.definition,
            self.feature_history.selected_body,
            self.feature_history.selected_feature,
        ) else {
            self.feature_history_error(self.catalog.text("feature-history-error-no-parameter"));
            return false;
        };
        let values = self
            .feature_history
            .move_input
            .split([',', ';'])
            .map(str::trim)
            .map(str::parse::<f64>)
            .collect::<Result<Vec<_>, _>>();
        let Ok(values) = values else {
            self.feature_history_error(self.catalog.text("feature-history-error-invalid-move"));
            return false;
        };
        let [dx, dy] = values.as_slice() else {
            self.feature_history_error(self.catalog.text("feature-history-error-invalid-move"));
            return false;
        };
        let request = BodyProfileTranslationRequest {
            definition_id,
            body_id,
            profile_id,
            delta_mm: [*dx, *dy],
        };
        let snapshot = self.document.current();
        let source = FeatureHistoryPreviewSource::ProfileTranslation {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            request,
        };
        let preview = match self.derive_feature_history_preview(&source) {
            Ok(preview) => preview,
            Err(error) => {
                self.feature_history_error(error);
                return false;
            }
        };
        let action = preview.action.clone();
        self.feature_history.preview = Some(preview);
        self.digest = self.catalog.format(
            "feature-history-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    fn prepare_feature_history_mutation(&mut self, mutation: BodyHistoryMutation) -> bool {
        let (Some(definition_id), Some(body_id)) = (
            self.feature_history.definition,
            self.feature_history.selected_body,
        ) else {
            self.feature_history_error(self.catalog.text("feature-history-error-no-body"));
            return false;
        };
        let request = BodyHistoryMutationRequest {
            definition_id,
            body_id,
            mutation,
        };
        let snapshot = self.document.current();
        let fork = match self.feature_history.change_scope {
            FeatureHistoryChangeScope::Shared => None,
            FeatureHistoryChangeScope::MakeUnique => {
                let Some(occurrence_id) = self.selected_fork_occurrence(definition_id) else {
                    self.feature_history_error(
                        self.catalog.text("feature-history-error-select-occurrence"),
                    );
                    return false;
                };
                Some((
                    occurrence_id,
                    self.fork_definition_name(&snapshot, definition_id),
                ))
            }
            FeatureHistoryChangeScope::ReplaceComponent => return false,
        };
        let source = FeatureHistoryPreviewSource::Mutation {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest().to_owned(),
            request,
            fork,
        };
        let preview = match self.derive_feature_history_preview(&source) {
            Ok(preview) => preview,
            Err(error) => {
                self.feature_history_error(error);
                return false;
            }
        };
        let action = preview.action.clone();
        self.feature_history.preview = Some(preview);
        self.digest = self.catalog.format(
            "feature-history-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    fn prepare_component_replacement(&mut self) -> bool {
        let Some(source_definition_id) = self.feature_history.definition else {
            self.feature_history_error(
                self.catalog.text("feature-history-error-select-occurrence"),
            );
            return false;
        };
        let Some(selected_occurrence_id) = self.selected_fork_occurrence(source_definition_id)
        else {
            self.feature_history_error(
                self.catalog.text("feature-history-error-select-occurrence"),
            );
            return false;
        };
        let Some(target_definition_id) = self.feature_history.replacement_target else {
            self.feature_history_error(
                self.catalog
                    .text("feature-history-error-no-replacement-target"),
            );
            return false;
        };
        let snapshot = self.document.current();
        let missing_target_bodies = snapshot
            .definition(target_definition_id)
            .into_iter()
            .flat_map(|definition| definition.bodies())
            .filter(|body| body.visible() && body.consumed_by().is_none())
            .filter_map(|body| {
                self.exact_results
                    .get_body(&snapshot, target_definition_id, body.id())
                    .ok()
                    .flatten()
                    .is_none()
                    .then_some(body.id())
            })
            .collect::<Vec<_>>();
        if !missing_target_bodies.is_empty() {
            let executable = match self.exact_worker_executable() {
                Ok(executable) => executable,
                Err(error) => {
                    self.feature_history_error(error);
                    return false;
                }
            };
            let mut worker = match ExactWorkerSupervisor::spawn(executable) {
                Ok(worker) => worker,
                Err(error) => {
                    self.feature_history_error(error);
                    return false;
                }
            };
            let mut packages = Vec::with_capacity(missing_target_bodies.len());
            for body_id in missing_target_bodies {
                let request = match ExactFeatureChainRequest::from_snapshot_for_body(
                    &snapshot,
                    target_definition_id,
                    body_id,
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        self.feature_history_error(error);
                        return false;
                    }
                };
                let package = match worker.evaluate_rectangle(&request) {
                    Ok(package) => Arc::new(ExactBodyPackage::from(package)),
                    Err(error) => {
                        self.feature_history_error(error);
                        return false;
                    }
                };
                packages.push(package);
            }
            self.exact_results = match ExactResultRegistry::publish_body_results(
                &snapshot,
                &self.exact_results,
                packages,
            ) {
                Ok(results) => results,
                Err(error) => {
                    self.feature_history_error(error);
                    return false;
                }
            };
        }
        let source =
            FeatureHistoryPreviewSource::Replacement(ComponentReplacementImpactRequest::new(
                &snapshot,
                selected_occurrence_id,
                target_definition_id,
            ));
        let preview = match self.derive_feature_history_preview(&source) {
            Ok(preview) => preview,
            Err(error) => {
                self.feature_history_error(error);
                return false;
            }
        };
        let action = preview.action.clone();
        self.feature_history.preview = Some(preview);
        self.digest = self.catalog.format(
            "feature-history-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    fn confirm_feature_history_preview(&mut self) -> bool {
        let Some(preview) = self.feature_history.preview.take() else {
            return false;
        };
        let rederived = self.derive_feature_history_preview(&preview.source);
        if rederived.as_ref() != Ok(&preview) {
            self.feature_history_error(
                rederived
                    .err()
                    .unwrap_or_else(|| "feature history preview plan changed".to_owned()),
            );
            return false;
        }
        let propagated = preview.execution.is_propagated();
        let result = match &preview.execution {
            FeatureHistoryExecutionPlan::Local(proposal) => self
                .document
                .commit_verified_proposal(proposal)
                .map(|_| ())
                .map_err(|error| error.to_string()),
            FeatureHistoryExecutionPlan::Replacement(impact) => {
                commit_component_replacement(&mut self.document, &mut self.exact_results, impact)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            FeatureHistoryExecutionPlan::Shared(_) | FeatureHistoryExecutionPlan::Fork(_) => {
                let executable = match self.exact_worker_executable() {
                    Ok(executable) => executable,
                    Err(error) => {
                        self.feature_history_error(error);
                        return false;
                    }
                };
                let mut worker = match ExactWorkerSupervisor::spawn(executable) {
                    Ok(worker) => worker,
                    Err(error) => {
                        self.feature_history_error(error);
                        return false;
                    }
                };
                if let Some(task) = self.exact_task.take() {
                    task.cancelled.store(true, Ordering::Release);
                }
                match &preview.execution {
                    FeatureHistoryExecutionPlan::Shared(impact) => commit_shared_definition_change(
                        &mut self.document,
                        &mut self.exact_results,
                        impact,
                        |request| {
                            worker
                                .evaluate_rectangle(request)
                                .map(ExactBodyPackage::from)
                                .map(Arc::new)
                                .map_err(|error| error.to_string())
                        },
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                    FeatureHistoryExecutionPlan::Fork(impact) => commit_occurrence_fork_change(
                        &mut self.document,
                        &mut self.exact_results,
                        impact,
                        |request| {
                            worker
                                .evaluate_rectangle(request)
                                .map(ExactBodyPackage::from)
                                .map(Arc::new)
                                .map_err(|error| error.to_string())
                        },
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                    FeatureHistoryExecutionPlan::Local(_)
                    | FeatureHistoryExecutionPlan::Replacement(_) => unreachable!(),
                }
            }
        };
        match result {
            Ok(()) => {
                self.feature_history.parameter_source = None;
                self.reconcile_selection();
                if propagated {
                    let snapshot = self.document.current();
                    self.render_plan = Some(Arc::new(InstancedRenderPlan::from_snapshot(
                        &snapshot,
                        &self.exact_results,
                        &mut self.render_cache,
                    )));
                    self.interaction_projection_cache.get_mut().take();
                    self.exact_source = Some((
                        snapshot.document_id(),
                        snapshot.revision_id(),
                        snapshot.canonical_digest(),
                    ));
                    self.exact_retry_at = None;
                }
                self.digest = self.catalog.format(
                    "feature-history-committed",
                    &BTreeMap::from([("action", preview.action)]),
                );
                true
            }
            Err(error) => {
                self.feature_history_error(error);
                false
            }
        }
    }

    pub(super) fn cancel_feature_history_preview(&mut self) {
        self.feature_history.preview = None;
        self.digest = self.catalog.text("feature-history-cancelled");
    }

    fn show_shared_change_impact(&self, ui: &mut egui::Ui, impact: &SharedChangeImpactProjection) {
        ui.label(self.catalog.text("feature-history-shared-impact-title"));
        let occurrences = impact
            .occurrences
            .iter()
            .map(|occurrence| {
                self.catalog.format(
                    "feature-history-shared-impact-occurrence",
                    &BTreeMap::from([
                        ("id", occurrence.occurrence_id.0.to_string()),
                        (
                            "visibility",
                            self.catalog.text(if occurrence.visible {
                                "feature-history-shared-impact-visible"
                            } else {
                                "feature-history-shared-impact-hidden"
                            }),
                        ),
                    ]),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-occurrences",
            &BTreeMap::from([("items", occurrences)]),
        ));
        ui.label(self.catalog.format(
            "feature-history-shared-impact-bodies",
            &BTreeMap::from([("items", body_ids(&impact.affected_body_ids))]),
        ));
        let mates = impact
            .mate_references
            .iter()
            .map(|reference| format!("{}@{}", reference.mate_id.0, reference.occurrence_id.0))
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-mates",
            &BTreeMap::from([(
                "items",
                if mates.is_empty() {
                    "—".to_owned()
                } else {
                    mates
                },
            )]),
        ));
        let views = impact
            .drawing_views
            .iter()
            .map(|view| {
                let kind = self.catalog.text(match view.view {
                    ketchup_core::drawing::OrthographicViewKind::Front => {
                        "feature-history-shared-impact-view-front"
                    }
                    ketchup_core::drawing::OrthographicViewKind::Top => {
                        "feature-history-shared-impact-view-top"
                    }
                    ketchup_core::drawing::OrthographicViewKind::Right => {
                        "feature-history-shared-impact-view-right"
                    }
                });
                format!("{}:{kind}", view.sheet_id.0)
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-views",
            &BTreeMap::from([(
                "items",
                if views.is_empty() {
                    "—".to_owned()
                } else {
                    views
                },
            )]),
        ));
        let jobs = impact
            .exact_jobs
            .iter()
            .map(|job| format!("{}:{}", job.body_id.0, job.producer_feature_id.0))
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-jobs",
            &BTreeMap::from([(
                "items",
                if jobs.is_empty() {
                    "—".to_owned()
                } else {
                    jobs
                },
            )]),
        ));
        let exports = impact
            .exports
            .iter()
            .map(|export| {
                let format = self.catalog.text(match export.format {
                    SharedChangeExportFormat::Step => "feature-history-shared-impact-export-step",
                    SharedChangeExportFormat::Stl => "feature-history-shared-impact-export-stl",
                });
                let eligibility = self.catalog.text(match export.eligibility {
                    SharedChangeExportEligibility::PendingExactRecompute => {
                        "feature-history-shared-impact-export-pending"
                    }
                    SharedChangeExportEligibility::CurrentExact => {
                        "feature-history-shared-impact-export-current"
                    }
                });
                format!("{format}: {eligibility}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-exports",
            &BTreeMap::from([(
                "items",
                if exports.is_empty() {
                    self.catalog
                        .text("feature-history-shared-impact-export-none")
                } else {
                    exports
                },
            )]),
        ));
        ui.label(self.catalog.format(
            "feature-history-shared-impact-diagnostics",
            &BTreeMap::from([
                ("revision", impact.source_revision.to_string()),
                ("definition", impact.definition_id.0.to_string()),
            ]),
        ));
    }

    fn show_occurrence_fork_impact(
        &self,
        ui: &mut egui::Ui,
        impact: &OccurrenceForkImpactProjection,
    ) {
        ui.label(self.catalog.text("feature-history-fork-impact-title"));
        ui.label(self.catalog.format(
            "feature-history-fork-impact-identity",
            &BTreeMap::from([
                ("occurrence", impact.selected_occurrence_id.0.to_string()),
                ("source", impact.source_definition_id.0.to_string()),
                ("fork", impact.fork_definition_id.0.to_string()),
            ]),
        ));
        let siblings = impact
            .unchanged_sibling_occurrences
            .iter()
            .map(|occurrence| {
                self.catalog.format(
                    "feature-history-shared-impact-occurrence",
                    &BTreeMap::from([
                        ("id", occurrence.occurrence_id.0.to_string()),
                        (
                            "visibility",
                            self.catalog.text(if occurrence.visible {
                                "feature-history-shared-impact-visible"
                            } else {
                                "feature-history-shared-impact-hidden"
                            }),
                        ),
                    ]),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-fork-impact-siblings",
            &BTreeMap::from([(
                "items",
                if siblings.is_empty() {
                    "—".to_owned()
                } else {
                    siblings
                },
            )]),
        ));
        ui.label(self.catalog.format(
            "feature-history-shared-impact-bodies",
            &BTreeMap::from([("items", body_ids(&impact.affected_fork_body_ids))]),
        ));
        let mates = impact
            .mate_references
            .iter()
            .map(|reference| format!("{}@{}", reference.mate_id.0, reference.occurrence_id.0))
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-mates",
            &BTreeMap::from([(
                "items",
                if mates.is_empty() {
                    "—".to_owned()
                } else {
                    mates
                },
            )]),
        ));
        let views = impact
            .drawing_views
            .iter()
            .map(|view| {
                let kind = self.catalog.text(match view.view {
                    ketchup_core::drawing::OrthographicViewKind::Front => {
                        "feature-history-shared-impact-view-front"
                    }
                    ketchup_core::drawing::OrthographicViewKind::Top => {
                        "feature-history-shared-impact-view-top"
                    }
                    ketchup_core::drawing::OrthographicViewKind::Right => {
                        "feature-history-shared-impact-view-right"
                    }
                });
                format!("{}:{kind}", view.sheet_id.0)
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-views",
            &BTreeMap::from([(
                "items",
                if views.is_empty() {
                    "—".to_owned()
                } else {
                    views
                },
            )]),
        ));
        let jobs = impact
            .exact_jobs
            .iter()
            .map(|job| format!("{}:{}", job.body_id.0, job.producer_feature_id.0))
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-jobs",
            &BTreeMap::from([(
                "items",
                if jobs.is_empty() {
                    "—".to_owned()
                } else {
                    jobs
                },
            )]),
        ));
        let exports = impact
            .exports
            .iter()
            .map(|export| {
                let format = self.catalog.text(match export.format {
                    SharedChangeExportFormat::Step => "feature-history-shared-impact-export-step",
                    SharedChangeExportFormat::Stl => "feature-history-shared-impact-export-stl",
                });
                let eligibility = self.catalog.text(match export.eligibility {
                    SharedChangeExportEligibility::PendingExactRecompute => {
                        "feature-history-shared-impact-export-pending"
                    }
                    SharedChangeExportEligibility::CurrentExact => {
                        "feature-history-shared-impact-export-current"
                    }
                });
                format!("{format}: {eligibility}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-exports",
            &BTreeMap::from([(
                "items",
                if exports.is_empty() {
                    self.catalog
                        .text("feature-history-shared-impact-export-none")
                } else {
                    exports
                },
            )]),
        ));
        ui.label(self.catalog.format(
            "feature-history-fork-impact-diagnostics",
            &BTreeMap::from([
                ("revision", impact.source_revision.to_string()),
                ("source", impact.source_definition_id.0.to_string()),
                ("fork", impact.fork_definition_id.0.to_string()),
            ]),
        ));
    }

    fn show_component_replacement_impact(
        &self,
        ui: &mut egui::Ui,
        impact: &ComponentReplacementImpactProjection,
    ) {
        ui.label(
            self.catalog
                .text("feature-history-replacement-impact-title"),
        );
        ui.label(self.catalog.format(
            "feature-history-replacement-impact-identity",
            &BTreeMap::from([
                ("occurrence", impact.selected_occurrence_id.0.to_string()),
                ("source", impact.source_definition_id.0.to_string()),
                ("target", impact.target_definition_id.0.to_string()),
            ]),
        ));
        let bodies = impact
            .body_correspondence
            .iter()
            .map(|mapping| format!("{}→{}", mapping.source_body_id.0, mapping.target_body_id.0))
            .collect::<Vec<_>>()
            .join(", ");
        let features = impact
            .feature_correspondence
            .iter()
            .map(|mapping| {
                format!(
                    "{}→{}",
                    mapping.source_feature_id.0, mapping.target_feature_id.0
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let subshapes = impact
            .subshape_correspondence
            .iter()
            .map(|mapping| {
                format!(
                    "{}:{}→{}",
                    mapping.semantic_role,
                    mapping.source_producer_feature_id.0,
                    mapping.target_producer_feature_id.0
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-replacement-impact-bodies",
            &BTreeMap::from([("items", bodies)]),
        ));
        ui.label(self.catalog.format(
            "feature-history-replacement-impact-features",
            &BTreeMap::from([("items", features)]),
        ));
        ui.label(self.catalog.format(
            "feature-history-replacement-impact-subshapes",
            &BTreeMap::from([("items", subshapes)]),
        ));
        let siblings = impact
            .unchanged_source_occurrences
            .iter()
            .map(|occurrence| format!("S{}", occurrence.occurrence_id.0))
            .chain(
                impact
                    .unchanged_target_occurrences
                    .iter()
                    .map(|occurrence| format!("T{}", occurrence.occurrence_id.0)),
            )
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-replacement-impact-siblings",
            &BTreeMap::from([(
                "items",
                if siblings.is_empty() {
                    "—".to_owned()
                } else {
                    siblings
                },
            )]),
        ));
        let mates = impact
            .mate_references
            .iter()
            .map(|reference| format!("{}@{}", reference.mate_id.0, reference.occurrence_id.0))
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-mates",
            &BTreeMap::from([(
                "items",
                if mates.is_empty() {
                    "—".to_owned()
                } else {
                    mates
                },
            )]),
        ));
        let views = impact
            .drawing_views
            .iter()
            .map(|view| {
                let kind = self.catalog.text(match view.view {
                    ketchup_core::drawing::OrthographicViewKind::Front => {
                        "feature-history-shared-impact-view-front"
                    }
                    ketchup_core::drawing::OrthographicViewKind::Top => {
                        "feature-history-shared-impact-view-top"
                    }
                    ketchup_core::drawing::OrthographicViewKind::Right => {
                        "feature-history-shared-impact-view-right"
                    }
                });
                format!("{}:{kind}", view.sheet_id.0)
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-views",
            &BTreeMap::from([(
                "items",
                if views.is_empty() {
                    "—".to_owned()
                } else {
                    views
                },
            )]),
        ));
        let jobs = impact
            .exact_jobs
            .iter()
            .map(|job| {
                format!(
                    "{}:{}:{}",
                    job.definition_id.0, job.body_id.0, job.producer_feature_id.0
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-replacement-impact-jobs",
            &BTreeMap::from([("items", jobs)]),
        ));
        let exports = impact
            .exports
            .iter()
            .map(|export| {
                let format = self.catalog.text(match export.format {
                    SharedChangeExportFormat::Step => "feature-history-shared-impact-export-step",
                    SharedChangeExportFormat::Stl => "feature-history-shared-impact-export-stl",
                });
                let eligibility = self.catalog.text(match export.eligibility {
                    SharedChangeExportEligibility::PendingExactRecompute => {
                        "feature-history-shared-impact-export-pending"
                    }
                    SharedChangeExportEligibility::CurrentExact => {
                        "feature-history-shared-impact-export-current"
                    }
                });
                format!("{format}: {eligibility}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        ui.label(self.catalog.format(
            "feature-history-shared-impact-exports",
            &BTreeMap::from([("items", exports)]),
        ));
        ui.label(
            self.catalog.format(
                "feature-history-replacement-impact-diagnostics",
                &BTreeMap::from([
                    ("revision", impact.source_revision.to_string()),
                    (
                        "candidate",
                        impact
                            .candidate_digest
                            .clone()
                            .unwrap_or_else(|| "—".to_owned()),
                    ),
                ]),
            ),
        );
    }

    pub(super) fn show_feature_history(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.catalog.text("feature-history-title"))
            .id_salt("feature-history")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("feature-history-scroll")
                    .max_height(700.0)
                    .show(ui, |ui| self.show_feature_history_content(ui));
            });
        ui.separator();
    }

    fn show_feature_history_content(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        let Some(projection) = self.reconcile_feature_history(&snapshot) else {
            ui.label(self.catalog.text("feature-history-unavailable"));
            return;
        };
        let mut action = None;
        let definitions = snapshot
            .definitions()
            .map(|definition| (definition.id(), definition.name().to_owned()))
            .collect::<Vec<_>>();
        let replacement_targets = definitions
            .iter()
            .filter(|(id, _)| Some(*id) != self.feature_history.definition)
            .cloned()
            .collect::<Vec<_>>();
        if self
            .feature_history
            .replacement_target
            .is_none_or(|id| !replacement_targets.iter().any(|(target, _)| *target == id))
        {
            self.feature_history.replacement_target =
                replacement_targets.first().map(|(id, _)| *id);
        }
        if let Some(mut selected) = self.feature_history.definition {
            let label = self.catalog.text("feature-history-definition");
            let selected_text = definitions
                .iter()
                .find(|(id, _)| *id == selected)
                .map_or_else(|| selected.0.to_string(), |(_, name)| name.clone());
            let response = egui::ComboBox::from_id_salt("feature-history-definition")
                .width(ui.available_width())
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (id, name) in &definitions {
                        ui.selectable_value(&mut selected, *id, name);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
            if self.feature_history.definition != Some(selected) {
                action = Some(FeatureHistoryUiAction::SelectDefinition(selected));
            }
        }

        ui.label(self.catalog.text("feature-history-bodies"));
        for body in &projection.bodies {
            let status = self.catalog.text(if body.consumed_by.is_some() {
                "feature-history-body-consumed"
            } else if body.active {
                "feature-history-body-active"
            } else if body.visible {
                "feature-history-body-visible"
            } else {
                "feature-history-body-hidden"
            });
            let label = self.catalog.format(
                "feature-history-select-body",
                &BTreeMap::from([
                    ("name", body.name.clone()),
                    ("id", body.body_id.0.to_string()),
                    ("status", status),
                ]),
            );
            if ui
                .selectable_label(
                    self.feature_history.selected_body == Some(body.body_id),
                    label,
                )
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::SelectBody(body.body_id));
            }
        }

        let Some(body) = projection
            .bodies
            .iter()
            .find(|body| Some(body.body_id) == self.feature_history.selected_body)
        else {
            return;
        };
        ui.separator();
        ui.label(self.catalog.text("feature-history-features"));
        for entry in &body.features {
            let status = self.catalog.text(match entry.state {
                FeatureHistoryState::Active => "feature-history-state-active",
                FeatureHistoryState::RollbackSuppressed => "feature-history-state-suppressed",
            });
            let label = self.catalog.format(
                "feature-history-select-feature",
                &BTreeMap::from([
                    ("name", entry.name.clone()),
                    ("id", entry.feature_id.0.to_string()),
                    ("status", status),
                ]),
            );
            if ui
                .selectable_label(
                    self.feature_history.selected_feature == Some(entry.feature_id),
                    label,
                )
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::SelectFeature(entry.feature_id));
            }
        }

        if let Some(selected) = projection.selected_feature.as_ref()
            && let Some(entry) = body
                .features
                .iter()
                .find(|entry| entry.feature_id == selected.feature_id)
        {
            ui.separator();
            ui.label(
                self.catalog.format(
                    "feature-history-provenance",
                    &BTreeMap::from([
                        ("feature", selected.feature_id.0.to_string()),
                        ("index", entry.topological_index.to_string()),
                        ("dependencies", ids(&entry.dependencies)),
                        ("dependents", ids(&entry.dependents)),
                        ("inputs", body_ids(&selected.input_body_ids)),
                        (
                            "output",
                            selected
                                .output_body_id
                                .map_or_else(|| "—".to_owned(), |id| id.0.to_string()),
                        ),
                    ]),
                ),
            );
        }
        if let Some(subshape) = projection.selected_subshape.as_ref() {
            ui.label(self.catalog.format(
                "feature-history-subshape-provenance",
                &BTreeMap::from([
                    ("role", subshape.semantic_role.clone()),
                    ("source", subshape.source_element_id.clone()),
                    ("lineage", subshape.lineage_digest.clone()),
                ]),
            ));
        }

        let definition_reused = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| {
                occurrence.instance_path.is_root()
                    && occurrence.definition_id == projection.definition_id
            })
            .count()
            >= 2;
        let can_replace = !replacement_targets.is_empty();
        if !definition_reused
            && self.feature_history.change_scope == FeatureHistoryChangeScope::MakeUnique
        {
            self.feature_history.change_scope = FeatureHistoryChangeScope::Shared;
        }
        if !can_replace
            && self.feature_history.change_scope == FeatureHistoryChangeScope::ReplaceComponent
        {
            self.feature_history.change_scope = FeatureHistoryChangeScope::Shared;
        }
        if self.feature_history.preview.is_none() && (definition_reused || can_replace) {
            ui.separator();
            ui.label(self.catalog.text("feature-history-change-scope"));
            ui.vertical(|ui| {
                ui.radio_value(
                    &mut self.feature_history.change_scope,
                    FeatureHistoryChangeScope::Shared,
                    self.catalog.text("feature-history-change-scope-shared"),
                );
                if definition_reused {
                    ui.radio_value(
                        &mut self.feature_history.change_scope,
                        FeatureHistoryChangeScope::MakeUnique,
                        self.catalog
                            .text("feature-history-change-scope-make-unique"),
                    );
                }
                if can_replace {
                    ui.radio_value(
                        &mut self.feature_history.change_scope,
                        FeatureHistoryChangeScope::ReplaceComponent,
                        self.catalog
                            .text("feature-history-change-scope-replace-component"),
                    );
                }
            });
            if matches!(
                self.feature_history.change_scope,
                FeatureHistoryChangeScope::MakeUnique | FeatureHistoryChangeScope::ReplaceComponent
            ) && self
                .selected_fork_occurrence(projection.definition_id)
                .is_none()
            {
                ui.label(
                    self.catalog
                        .text("feature-history-change-scope-select-occurrence"),
                );
            }
        }

        if let Some(preview) = self.feature_history.preview.as_ref() {
            ui.separator();
            ui.label(self.catalog.format(
                "feature-history-preview",
                &BTreeMap::from([
                    ("action", preview.action.clone()),
                    ("affected", ids(&preview.affected_feature_ids)),
                    ("suppressed", ids(&preview.suppressed_feature_ids)),
                ]),
            ));
            if let Some(impact) = preview.execution.shared_impact() {
                self.show_shared_change_impact(ui, impact);
            }
            if let Some(impact) = preview.execution.fork_impact() {
                self.show_occurrence_fork_impact(ui, impact);
            }
            if let Some(impact) = preview.execution.replacement_impact() {
                self.show_component_replacement_impact(ui, impact);
            }
            if ui
                .button(self.catalog.text("feature-history-confirm"))
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::Confirm);
            }
            if ui
                .button(self.catalog.text("feature-history-cancel"))
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::Cancel);
            }
        } else if self.feature_history.change_scope == FeatureHistoryChangeScope::ReplaceComponent {
            if let Some(mut target) = self.feature_history.replacement_target {
                let label = self.catalog.text("feature-history-replacement-target");
                let selected_text = replacement_targets
                    .iter()
                    .find(|(id, _)| *id == target)
                    .map_or_else(|| target.0.to_string(), |(_, name)| name.clone());
                let response = egui::ComboBox::from_id_salt("feature-history-replacement-target")
                    .width(ui.available_width())
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for (id, name) in &replacement_targets {
                            ui.selectable_value(&mut target, *id, name);
                        }
                    });
                response.response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
                });
                if self.feature_history.replacement_target != Some(target) {
                    action = Some(FeatureHistoryUiAction::SelectReplacementTarget(target));
                }
            }
            if ui
                .add_enabled(
                    self.feature_history.replacement_target.is_some()
                        && self
                            .selected_fork_occurrence(projection.definition_id)
                            .is_some(),
                    egui::Button::new(
                        self.catalog
                            .text("feature-history-preview-replace-component"),
                    ),
                )
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::PreviewReplacement);
            }
        } else if let Some(feature_id) = self.feature_history.selected_feature {
            let is_profile = snapshot.feature(feature_id).is_some_and(|feature| {
                matches!(
                    feature.kind(),
                    FeatureKind::Sketch(_)
                        | FeatureKind::Profile { .. }
                        | FeatureKind::SegmentProfile { .. }
                        | FeatureKind::SplineProfile { .. }
                )
            });
            let is_cut_profile = is_profile
                && snapshot.features().any(|feature| match feature.kind() {
                    FeatureKind::SketchPocket(spec) => spec.sketch == feature_id,
                    FeatureKind::ThroughCut { profile, .. }
                    | FeatureKind::Pocket { profile, .. } => *profile == feature_id,
                    FeatureKind::Boolean {
                        operation: BooleanOperation::Cut,
                        tool,
                        ..
                    } => snapshot.feature(*tool).is_some_and(|tool| {
                        matches!(
                            tool.kind(),
                            FeatureKind::Extrusion { profile, .. } if *profile == feature_id
                        )
                    }),
                    _ => false,
                });
            if is_cut_profile {
                if self.feature_history.move_input.is_empty() {
                    self.feature_history.move_input = "0, 0".to_owned();
                }
                let move_label = self.catalog.text("feature-history-move-profile-value");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.feature_history.move_input)
                        .hint_text(&move_label),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &move_label)
                });
                if ui
                    .button(self.catalog.text("feature-history-preview-move-profile"))
                    .clicked()
                {
                    action = Some(FeatureHistoryUiAction::PreviewMove);
                }
            }
            let choices = self.feature_parameter_choices(&snapshot, feature_id);
            if self
                .feature_history
                .selected_parameter
                .is_none_or(|target| !choices.iter().any(|choice| choice.target == target))
            {
                self.feature_history.selected_parameter =
                    choices.first().map(|choice| choice.target);
                self.feature_history.parameter_source = None;
            }
            if let Some(mut target) = self.feature_history.selected_parameter {
                let selected = choices.iter().find(|choice| choice.target == target);
                let label = self.catalog.text("feature-history-parameter");
                let selected_text = selected
                    .map(|choice| choice.label.clone())
                    .unwrap_or_else(|| self.catalog.text("feature-history-no-parameter"));
                let response = egui::ComboBox::from_id_salt("feature-history-parameter")
                    .width(ui.available_width())
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for choice in &choices {
                            ui.selectable_value(&mut target, choice.target, &choice.label);
                        }
                    });
                response.response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
                });
                if self.feature_history.selected_parameter != Some(target) {
                    action = Some(FeatureHistoryUiAction::SelectParameter(target));
                } else if self.feature_history.parameter_source
                    != Some((snapshot.revision_id(), target))
                    && let Some(choice) = selected
                {
                    self.feature_history.value_input = format_height(choice.value_mm);
                    self.feature_history.parameter_source = Some((snapshot.revision_id(), target));
                }
                let value_label = self.catalog.text("feature-history-exact-value");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.feature_history.value_input)
                        .hint_text(&value_label),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &value_label)
                });
                if ui
                    .add_enabled(
                        !choices.is_empty(),
                        egui::Button::new(self.catalog.text("feature-history-preview-edit")),
                    )
                    .clicked()
                {
                    action = Some(FeatureHistoryUiAction::PreviewEdit);
                }
            } else {
                ui.label(self.catalog.text("feature-history-no-parameter"));
            }

            if ui
                .button(self.catalog.text("feature-history-preview-suppress"))
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::PreviewSuppress);
            }
            let can_resume = snapshot
                .suppressed_feature_ids(projection.definition_id, body.body_id)
                .is_some_and(|features| !features.is_empty());
            if ui
                .add_enabled(
                    can_resume,
                    egui::Button::new(self.catalog.text("feature-history-preview-resume")),
                )
                .clicked()
            {
                action = Some(FeatureHistoryUiAction::PreviewResume);
            }
        }

        match action {
            Some(FeatureHistoryUiAction::SelectDefinition(id)) => {
                self.feature_history.definition = Some(id);
                self.feature_history.selected_body = None;
                self.feature_history.selected_feature = None;
                self.feature_history.selected_parameter = None;
                self.feature_history.parameter_source = None;
                self.feature_history.replacement_target = None;
            }
            Some(FeatureHistoryUiAction::SelectBody(id)) => {
                self.feature_history.selected_body = Some(id);
                self.feature_history.selected_feature = None;
                self.feature_history.selected_parameter = None;
                self.feature_history.parameter_source = None;
                self.digest = self.catalog.format(
                    "feature-history-selected-body",
                    &BTreeMap::from([("id", id.0.to_string())]),
                );
            }
            Some(FeatureHistoryUiAction::SelectFeature(id)) => {
                self.feature_history.selected_feature = Some(id);
                self.feature_history.selected_parameter = None;
                self.feature_history.parameter_source = None;
                self.digest = self.catalog.format(
                    "feature-history-selected-feature",
                    &BTreeMap::from([("id", id.0.to_string())]),
                );
            }
            Some(FeatureHistoryUiAction::SelectParameter(target)) => {
                self.feature_history.selected_parameter = Some(target);
                self.feature_history.parameter_source = None;
            }
            Some(FeatureHistoryUiAction::SelectReplacementTarget(target)) => {
                self.feature_history.replacement_target = Some(target);
            }
            Some(FeatureHistoryUiAction::PreviewEdit) => {
                self.prepare_feature_history_edit();
            }
            Some(FeatureHistoryUiAction::PreviewMove) => {
                self.prepare_feature_profile_translation();
            }
            Some(FeatureHistoryUiAction::PreviewSuppress) => {
                if let Some(feature_id) = self.feature_history.selected_feature {
                    self.prepare_feature_history_mutation(BodyHistoryMutation::SuppressFrom(
                        feature_id,
                    ));
                }
            }
            Some(FeatureHistoryUiAction::PreviewResume) => {
                self.prepare_feature_history_mutation(BodyHistoryMutation::Resume);
            }
            Some(FeatureHistoryUiAction::PreviewReplacement) => {
                self.prepare_component_replacement();
            }
            Some(FeatureHistoryUiAction::Confirm) => {
                self.confirm_feature_history_preview();
            }
            Some(FeatureHistoryUiAction::Cancel) => self.cancel_feature_history_preview(),
            None => {}
        }
    }

    #[must_use]
    pub fn feature_history_preview_pending(&self) -> bool {
        self.feature_history.preview.is_some()
    }

    #[must_use]
    pub fn feature_history_selected_body_id(&self) -> Option<ketchup_core::document::BodyId> {
        self.feature_history.selected_body
    }

    #[doc(hidden)]
    #[must_use]
    pub fn feature_history_selection_ids(
        &self,
    ) -> (
        Option<DefinitionId>,
        Option<ketchup_core::document::BodyId>,
        Option<DefinitionId>,
    ) {
        (
            self.feature_history.definition,
            self.feature_history.selected_body,
            self.feature_history.replacement_target,
        )
    }

    #[must_use]
    pub fn feature_history_selected_feature_id(&self) -> Option<FeatureId> {
        self.feature_history.selected_feature
    }

    #[doc(hidden)]
    #[must_use]
    pub fn feature_history_current_dependency_counts(&self) -> Option<[usize; 2]> {
        let snapshot = self.document.current();
        let definition_id = self.feature_history.definition?;
        let body_id = self.feature_history.selected_body?;
        let package = self
            .exact_results
            .get_body(&snapshot, definition_id, body_id)
            .ok()??;
        let fingerprint = &package.result_key().result_fingerprint;
        let mate_count = snapshot
            .assembly_mates()
            .filter(|mate| {
                [mate.endpoint_a(), mate.endpoint_b()]
                    .into_iter()
                    .any(|endpoint| endpoint.reference().definition_id == definition_id)
                    && [mate.endpoint_a(), mate.endpoint_b()]
                        .into_iter()
                        .filter(|endpoint| endpoint.reference().definition_id == definition_id)
                        .all(|endpoint| {
                            endpoint.health()
                                == ketchup_core::assembly::AssemblyReferenceHealth::Resolved
                                && endpoint.reference().result_fingerprint == *fingerprint
                        })
            })
            .count();
        let mut drawing_view_count = 0;
        for sheet in snapshot.drawing_sheets() {
            drawing_view_count += ketchup_core::drawing::project_orthographic_drawing(
                &snapshot,
                &self.exact_results,
                sheet,
            )
            .ok()?
            .views
            .len();
        }
        Some([mate_count, drawing_view_count])
    }

    #[must_use]
    pub fn feature_history_fork_impact_counts(&self) -> Option<[usize; 6]> {
        self.feature_history
            .preview
            .as_ref()
            .and_then(|preview| preview.execution.fork_impact())
            .map(|impact| {
                [
                    impact.unchanged_sibling_occurrences.len(),
                    impact.affected_fork_body_ids.len(),
                    impact.mate_references.len(),
                    impact.drawing_views.len(),
                    impact.exact_jobs.len(),
                    impact.exports.len(),
                ]
            })
    }

    #[must_use]
    pub fn feature_history_fork_identity(&self) -> Option<[u64; 3]> {
        self.feature_history
            .preview
            .as_ref()
            .and_then(|preview| preview.execution.fork_impact())
            .map(|impact| {
                [
                    impact.selected_occurrence_id.0,
                    impact.source_definition_id.0,
                    impact.fork_definition_id.0,
                ]
            })
    }

    #[must_use]
    pub fn feature_history_replacement_identity(&self) -> Option<[u64; 3]> {
        self.feature_history
            .preview
            .as_ref()
            .and_then(|preview| preview.execution.replacement_impact())
            .map(|impact| {
                [
                    impact.selected_occurrence_id.0,
                    impact.source_definition_id.0,
                    impact.target_definition_id.0,
                ]
            })
    }

    #[must_use]
    pub fn feature_history_replacement_impact_counts(&self) -> Option<[usize; 9]> {
        self.feature_history
            .preview
            .as_ref()
            .and_then(|preview| preview.execution.replacement_impact())
            .map(|impact| {
                [
                    impact.body_correspondence.len(),
                    impact.feature_correspondence.len(),
                    impact.subshape_correspondence.len(),
                    impact.unchanged_source_occurrences.len(),
                    impact.unchanged_target_occurrences.len(),
                    impact.mate_references.len(),
                    impact.drawing_views.len(),
                    impact.exact_jobs.len(),
                    impact.exports.len(),
                ]
            })
    }

    #[must_use]
    pub fn feature_history_shared_impact_counts(&self) -> Option<[usize; 6]> {
        self.feature_history
            .preview
            .as_ref()
            .and_then(|preview| preview.execution.shared_impact())
            .map(|impact| {
                [
                    impact.occurrences.len(),
                    impact.affected_body_ids.len(),
                    impact.mate_references.len(),
                    impact.drawing_views.len(),
                    impact.exact_jobs.len(),
                    impact.exports.len(),
                ]
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_mutation_preview(app: &KetchupApp, boundary: FeatureId) -> FeatureHistoryUiPreview {
        let snapshot = app.document.current();
        app.derive_feature_history_preview(&FeatureHistoryPreviewSource::Mutation {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest().to_owned(),
            request: BodyHistoryMutationRequest {
                definition_id: DefinitionId(1),
                body_id: ketchup_core::document::BodyId(1),
                mutation: BodyHistoryMutation::SuppressFrom(boundary),
            },
            fork: None,
        })
        .expect("initial feature suppression is previewable")
    }

    #[test]
    fn exact_preview_source_rejects_plan_metadata_tamper_stale_and_replay_atomically() {
        let mut valid = KetchupApp::new();
        let initial_revision = valid.document_revision();
        let initial_digest = valid.canonical_digest();
        let initial_undo = valid.undo_step_count();
        let preview = local_mutation_preview(&valid, FeatureId(2));
        valid.feature_history.preview = Some(preview.clone());

        assert!(valid.confirm_feature_history_preview());
        assert_eq!(valid.document_revision(), initial_revision + 1);
        assert_eq!(valid.undo_step_count(), initial_undo + 1);
        let committed_digest = valid.canonical_digest();
        assert_ne!(committed_digest, initial_digest);

        valid.feature_history.preview = Some(preview);
        assert!(!valid.confirm_feature_history_preview());
        assert_eq!(valid.canonical_digest(), committed_digest);
        assert_eq!(valid.undo_step_count(), initial_undo + 1);
        assert!(valid.undo());
        assert_eq!(valid.canonical_digest(), initial_digest);
        assert!(valid.redo());
        assert_eq!(valid.canonical_digest(), committed_digest);

        let mut tampered = KetchupApp::new();
        let before_tamper = (
            tampered.document_revision(),
            tampered.canonical_digest(),
            tampered.undo_step_count(),
        );
        let mut tampered_preview = local_mutation_preview(&tampered, FeatureId(2));
        tampered_preview.kind = FeatureHistoryPreviewKind::Resume;
        tampered.feature_history.preview = Some(tampered_preview);
        assert!(!tampered.confirm_feature_history_preview());
        assert_eq!(
            (
                tampered.document_revision(),
                tampered.canonical_digest(),
                tampered.undo_step_count(),
            ),
            before_tamper
        );

        let base = local_mutation_preview(&tampered, FeatureId(2));
        let mut source_tampered = base.clone();
        let FeatureHistoryPreviewSource::Mutation { request, .. } = &mut source_tampered.source
        else {
            unreachable!()
        };
        request.mutation = BodyHistoryMutation::SuppressFrom(FeatureId(1));

        let mut execution_tampered = base.clone();
        execution_tampered.execution = local_mutation_preview(&tampered, FeatureId(1)).execution;

        let mut action_tampered = base.clone();
        action_tampered.action.push_str(" changed");

        let mut metadata_tampered = base;
        metadata_tampered
            .affected_feature_ids
            .push(FeatureId(u64::MAX));

        for preview in [
            source_tampered,
            execution_tampered,
            action_tampered,
            metadata_tampered,
        ] {
            tampered.feature_history.preview = Some(preview);
            assert!(!tampered.confirm_feature_history_preview());
            assert_eq!(
                (
                    tampered.document_revision(),
                    tampered.canonical_digest(),
                    tampered.undo_step_count(),
                ),
                before_tamper
            );
        }

        let mut stale = KetchupApp::new();
        let stale_preview = local_mutation_preview(&stale, FeatureId(2));
        stale
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceVisibility {
                    id: OccurrenceId(1),
                    visible: false,
                },
            ]))
            .expect("drift mutation is valid");
        let after_drift = (
            stale.document_revision(),
            stale.canonical_digest(),
            stale.undo_step_count(),
        );
        stale.feature_history.preview = Some(stale_preview);
        assert!(!stale.confirm_feature_history_preview());
        assert_eq!(
            (
                stale.document_revision(),
                stale.canonical_digest(),
                stale.undo_step_count(),
            ),
            after_drift
        );
    }
}
