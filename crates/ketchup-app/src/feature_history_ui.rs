use super::*;
use ketchup_core::feature_history::{
    BodyHistoryMutation, BodyHistoryMutationRequest, ExactParameterEdit, ExactParameterEditTarget,
    FeatureHistoryProjection, FeatureHistoryQuery, FeatureHistoryState, RollbackPreviewRequest,
    prepare_body_history_mutation, prepare_body_parameter_edit, project_feature_history,
};
use ketchup_core::shared_change::{
    SharedChangeExportEligibility, SharedChangeExportFormat, SharedChangeImpactError,
    SharedChangeImpactProjection, SharedDefinitionChangeRequest, commit_shared_definition_change,
    project_shared_change_impact,
};
use ketchup_core::sketch::SketchConstraintKind;

#[derive(Clone, Debug)]
enum FeatureHistoryPreviewKind {
    ExactEdit,
    Suppress { boundary: FeatureId },
    Resume,
}

#[derive(Clone, Debug)]
struct FeatureHistoryUiPreview {
    proposal: Proposal,
    kind: FeatureHistoryPreviewKind,
    action: String,
    affected_feature_ids: Vec<FeatureId>,
    suppressed_feature_ids: Vec<FeatureId>,
    shared_impact: Option<SharedChangeImpactProjection>,
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
    preview: Option<FeatureHistoryUiPreview>,
}

enum FeatureHistoryUiAction {
    SelectDefinition(DefinitionId),
    SelectBody(ketchup_core::document::BodyId),
    SelectFeature(FeatureId),
    SelectParameter(ExactParameterEditTarget),
    PreviewEdit,
    PreviewSuppress,
    PreviewResume,
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
            FeatureKind::Pad(spec) => vec![ParameterChoice {
                target: ExactParameterEditTarget::FeatureDimension(feature_id),
                label: self.catalog.text("feature-history-parameter-extent"),
                value_mm: spec.extent.distance().millimetres(),
            }],
            FeatureKind::SketchPocket(spec) => vec![ParameterChoice {
                target: ExactParameterEditTarget::FeatureDimension(feature_id),
                label: self.catalog.text("feature-history-parameter-depth"),
                value_mm: spec.extent.distance().millimetres(),
            }],
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
                    FeatureHistoryPreviewKind::ExactEdit | FeatureHistoryPreviewKind::Resume => {
                        None
                    }
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
        let (proposal, affected_feature_ids, shared_impact) = match project_shared_change_impact(
            &self.document,
            &self.exact_results,
            SharedDefinitionChangeRequest::exact_parameter_edit(
                &self.document.current(),
                request.clone(),
            ),
            ProposalPrincipal::ManualClient,
        ) {
            Ok(impact) => (
                impact.proposal.clone(),
                impact.affected_feature_ids.clone(),
                Some(impact),
            ),
            Err(SharedChangeImpactError::DefinitionNotReused(_)) => {
                let preview = match prepare_body_parameter_edit(
                    &self.document,
                    request,
                    ProposalPrincipal::ManualClient,
                ) {
                    Ok(preview) => preview,
                    Err(error) => {
                        self.feature_history_error(error);
                        return false;
                    }
                };
                (preview.proposal, preview.affected_feature_ids, None)
            }
            Err(error) => {
                self.feature_history_error(error);
                return false;
            }
        };
        if let Err(error) = self.document.preview_batch(proposal.batch()) {
            self.feature_history_error(error);
            return false;
        }
        let action = self.catalog.text("feature-history-action-edit");
        self.feature_history.preview = Some(FeatureHistoryUiPreview {
            proposal,
            kind: FeatureHistoryPreviewKind::ExactEdit,
            action: action.clone(),
            affected_feature_ids,
            suppressed_feature_ids: Vec::new(),
            shared_impact,
        });
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
        let kind = match mutation {
            BodyHistoryMutation::SuppressFrom(boundary) => {
                FeatureHistoryPreviewKind::Suppress { boundary }
            }
            BodyHistoryMutation::Resume => FeatureHistoryPreviewKind::Resume,
        };
        let request = BodyHistoryMutationRequest {
            definition_id,
            body_id,
            mutation,
        };
        let (proposal, affected_feature_ids, suppressed_feature_ids, shared_impact) =
            match project_shared_change_impact(
                &self.document,
                &self.exact_results,
                SharedDefinitionChangeRequest::body_history_mutation(
                    &self.document.current(),
                    request,
                ),
                ProposalPrincipal::ManualClient,
            ) {
                Ok(impact) => (
                    impact.proposal.clone(),
                    impact.affected_feature_ids.clone(),
                    match kind {
                        FeatureHistoryPreviewKind::Suppress { boundary } => impact
                            .affected_feature_ids
                            .iter()
                            .copied()
                            .filter(|feature_id| *feature_id >= boundary)
                            .collect(),
                        FeatureHistoryPreviewKind::Resume
                        | FeatureHistoryPreviewKind::ExactEdit => Vec::new(),
                    },
                    Some(impact),
                ),
                Err(SharedChangeImpactError::DefinitionNotReused(_)) => {
                    let preview = match prepare_body_history_mutation(
                        &self.document,
                        request,
                        ProposalPrincipal::ManualClient,
                    ) {
                        Ok(preview) => preview,
                        Err(error) => {
                            self.feature_history_error(error);
                            return false;
                        }
                    };
                    (
                        preview.proposal,
                        preview.affected_feature_ids,
                        preview.suppressed_feature_ids,
                        None,
                    )
                }
                Err(error) => {
                    self.feature_history_error(error);
                    return false;
                }
            };
        if let Err(error) = self.document.preview_batch(proposal.batch()) {
            self.feature_history_error(error);
            return false;
        }
        let action = self.catalog.text(match kind {
            FeatureHistoryPreviewKind::Suppress { .. } => "feature-history-action-suppress",
            FeatureHistoryPreviewKind::Resume => "feature-history-action-resume",
            FeatureHistoryPreviewKind::ExactEdit => unreachable!(),
        });
        self.feature_history.preview = Some(FeatureHistoryUiPreview {
            proposal,
            kind,
            action: action.clone(),
            affected_feature_ids,
            suppressed_feature_ids,
            shared_impact,
        });
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
        let result = if let Some(impact) = preview.shared_impact.as_ref() {
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
            commit_shared_definition_change(
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
            .map_err(|error| error.to_string())
        } else {
            self.document
                .commit_verified_proposal(&preview.proposal)
                .map(|_| ())
                .map_err(|error| error.to_string())
        };
        match result {
            Ok(()) => {
                self.feature_history.parameter_source = None;
                self.reconcile_selection();
                if preview.shared_impact.is_some() {
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
            if let Some(impact) = preview.shared_impact.as_ref() {
                self.show_shared_change_impact(ui, impact);
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
        } else if let Some(feature_id) = self.feature_history.selected_feature {
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
            Some(FeatureHistoryUiAction::PreviewEdit) => {
                self.prepare_feature_history_edit();
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
    pub fn feature_history_shared_impact_counts(&self) -> Option<[usize; 6]> {
        self.feature_history
            .preview
            .as_ref()
            .and_then(|preview| preview.shared_impact.as_ref())
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
