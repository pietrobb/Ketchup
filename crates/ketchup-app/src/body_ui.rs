use super::*;
use ketchup_core::document::{BodyId, MultiBodyBooleanPlan, NewBodyFeaturePlan, ToolBodyPolicy};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum BodyBooleanChoice {
    Cut,
    #[default]
    Union,
    Intersect,
}

impl BodyBooleanChoice {
    const ALL: [Self; 3] = [Self::Cut, Self::Union, Self::Intersect];

    const fn operation(self) -> BooleanOperation {
        match self {
            Self::Cut => BooleanOperation::Cut,
            Self::Union => BooleanOperation::Union,
            Self::Intersect => BooleanOperation::Intersect,
        }
    }

    const fn label_key(self) -> &'static str {
        match self {
            Self::Cut => "body-boolean-cut",
            Self::Union => "body-boolean-union",
            Self::Intersect => "body-boolean-intersect",
        }
    }
}

struct BodyProposalPreview {
    proposal: Proposal,
    action: String,
}

#[derive(Default)]
pub(super) struct BodyEditorState {
    definition: Option<DefinitionId>,
    selected_body: Option<BodyId>,
    source_feature: Option<FeatureId>,
    target_body: Option<BodyId>,
    tool_body: Option<BodyId>,
    body_name: String,
    feature_name: String,
    operation: BodyBooleanChoice,
    consume_tool: bool,
    preview: Option<BodyProposalPreview>,
}

enum BodyUiAction {
    Select(BodyId),
    Activate(BodyId),
    Visibility(BodyId, bool),
    Create,
    Combine,
}

impl KetchupApp {
    fn body_error(&mut self, reason: impl ToString) {
        self.digest = self.catalog.format(
            "body-error",
            &BTreeMap::from([("reason", reason.to_string())]),
        );
    }

    fn supported_body_features(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Vec<(FeatureId, String, FeatureKind)> {
        let Some(definition) = snapshot.definition(definition_id) else {
            return Vec::new();
        };
        definition
            .feature_ids()
            .iter()
            .filter_map(|id| {
                let feature = snapshot.feature(*id)?;
                matches!(
                    feature.kind(),
                    FeatureKind::Extrusion { .. } | FeatureKind::Pad(_)
                )
                .then(|| (*id, feature.name().to_owned(), feature.kind().clone()))
            })
            .collect()
    }

    fn terminal_body_feature(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        body_id: BodyId,
    ) -> Option<FeatureId> {
        let definition = snapshot.definition(definition_id)?;
        definition.feature_ids().iter().rev().copied().find(|id| {
            let Some(feature) = snapshot.feature(*id) else {
                return false;
            };
            if !matches!(
                feature.kind(),
                FeatureKind::Extrusion { .. } | FeatureKind::Pad(_)
            ) || definition
                .feature_body_ownership(*id)
                .and_then(|ownership| ownership.output_body_id())
                != Some(body_id)
            {
                return false;
            }
            !definition.feature_ids().iter().copied().any(|candidate| {
                candidate != *id
                    && snapshot.feature(candidate).is_some_and(|dependent| {
                        dependent.kind().dependencies().contains(id)
                            && definition
                                .feature_body_ownership(candidate)
                                .and_then(|ownership| ownership.output_body_id())
                                == Some(body_id)
                    })
            })
        })
    }

    fn reconcile_body_editor(&mut self, snapshot: &Snapshot) {
        let definitions = snapshot
            .definitions()
            .map(|definition| definition.id())
            .collect::<Vec<_>>();
        if self
            .body_editor
            .definition
            .is_none_or(|id| !definitions.contains(&id))
        {
            self.body_editor.definition = definitions.first().copied();
            self.body_editor.selected_body = None;
            self.body_editor.source_feature = None;
            self.body_editor.target_body = None;
            self.body_editor.tool_body = None;
            self.body_editor.body_name.clear();
            self.body_editor.feature_name.clear();
        }
        let Some(definition_id) = self.body_editor.definition else {
            return;
        };
        let Some(definition) = snapshot.definition(definition_id) else {
            return;
        };
        let bodies = definition
            .bodies()
            .filter(|body| body.consumed_by().is_none())
            .map(|body| body.id())
            .collect::<Vec<_>>();
        if self
            .body_editor
            .selected_body
            .is_none_or(|id| !bodies.contains(&id))
        {
            self.body_editor.selected_body = Some(definition.active_body_id());
        }
        if self
            .body_editor
            .target_body
            .is_none_or(|id| !bodies.contains(&id))
        {
            self.body_editor.target_body = self.body_editor.selected_body;
        }
        if self
            .body_editor
            .tool_body
            .is_none_or(|id| !bodies.contains(&id) || Some(id) == self.body_editor.target_body)
        {
            self.body_editor.tool_body = bodies
                .iter()
                .copied()
                .find(|id| Some(*id) != self.body_editor.target_body);
        }
        let features = Self::supported_body_features(snapshot, definition_id);
        if self
            .body_editor
            .source_feature
            .is_none_or(|id| !features.iter().any(|(candidate, _, _)| *candidate == id))
        {
            self.body_editor.source_feature = features.first().map(|(id, _, _)| *id);
        }
        if self.body_editor.body_name.is_empty() {
            self.body_editor.body_name = self.catalog.format(
                "body-default-name",
                &BTreeMap::from([("number", (definition.bodies().count() + 1).to_string())]),
            );
        }
        if self.body_editor.feature_name.is_empty() {
            self.body_editor.feature_name = self.catalog.text("body-default-feature-name");
        }
    }

    fn prepare_body_preview(&mut self, proposal: Proposal, action: String) -> bool {
        if let Err(error) = self.document.preview_batch(proposal.batch()) {
            self.body_error(error);
            return false;
        }
        self.body_editor.preview = Some(BodyProposalPreview {
            proposal,
            action: action.clone(),
        });
        self.digest = self
            .catalog
            .format("body-preview-ready", &BTreeMap::from([("action", action)]));
        true
    }

    fn preview_body_command(&mut self, command: CanonicalCommand, action: String) -> bool {
        match self.document.plan_body_command(command) {
            Ok(proposal) => self.prepare_body_preview(proposal, action),
            Err(error) => {
                self.body_error(error);
                false
            }
        }
    }

    fn preview_new_body_feature(&mut self) -> bool {
        let snapshot = self.document.current();
        let (Some(definition_id), Some(source_feature_id)) =
            (self.body_editor.definition, self.body_editor.source_feature)
        else {
            self.body_error(self.catalog.text("body-error-source"));
            return false;
        };
        let Some(source) = snapshot.feature(source_feature_id) else {
            self.body_error(self.catalog.text("body-error-source"));
            return false;
        };
        let body_name = self.body_editor.body_name.trim().to_owned();
        let feature_name = self.body_editor.feature_name.trim().to_owned();
        if body_name.is_empty() || feature_name.is_empty() {
            self.body_error(self.catalog.text("body-error-name"));
            return false;
        }
        let body_id = BodyId(
            snapshot
                .definition(definition_id)
                .into_iter()
                .flat_map(|definition| definition.bodies())
                .map(|body| body.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let feature_id = FeatureId(
            snapshot
                .features()
                .map(|feature| feature.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let action = self.catalog.format(
            "body-action-create",
            &BTreeMap::from([("name", body_name.clone())]),
        );
        match self.document.plan_new_body_feature(
            NewBodyFeaturePlan {
                definition_id,
                body_id,
                body_name,
                feature_id,
                feature_name,
                feature_kind: source.kind().clone(),
            },
            ProposalContext::canonical_preview(),
        ) {
            Ok(proposal) => self.prepare_body_preview(proposal, action),
            Err(error) => {
                self.body_error(error);
                false
            }
        }
    }

    fn preview_multibody_boolean(&mut self) -> bool {
        let snapshot = self.document.current();
        let (Some(definition_id), Some(target_body_id), Some(tool_body_id)) = (
            self.body_editor.definition,
            self.body_editor.target_body,
            self.body_editor.tool_body,
        ) else {
            self.body_error(self.catalog.text("body-error-operands"));
            return false;
        };
        let (Some(target_feature_id), Some(tool_feature_id)) = (
            Self::terminal_body_feature(&snapshot, definition_id, target_body_id),
            Self::terminal_body_feature(&snapshot, definition_id, tool_body_id),
        ) else {
            self.body_error(self.catalog.text("body-error-terminal"));
            return false;
        };
        let result_feature_id = FeatureId(
            snapshot
                .features()
                .map(|feature| feature.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let operation = self.body_editor.operation;
        let action = self.catalog.format(
            "body-action-combine",
            &BTreeMap::from([("operation", self.catalog.text(operation.label_key()))]),
        );
        match self.document.plan_multibody_boolean(
            MultiBodyBooleanPlan {
                definition_id,
                operation: operation.operation(),
                target_body_id,
                target_feature_id,
                tool_body_id,
                tool_feature_id,
                result_feature_id,
                result_feature_name: self.catalog.format(
                    "body-default-result-name",
                    &BTreeMap::from([
                        ("operation", self.catalog.text(operation.label_key())),
                        ("number", result_feature_id.0.to_string()),
                    ]),
                ),
                tool_policy: if self.body_editor.consume_tool {
                    ToolBodyPolicy::Consume
                } else {
                    ToolBodyPolicy::Preserve
                },
            },
            ProposalContext::canonical_preview(),
        ) {
            Ok(proposal) => self.prepare_body_preview(proposal, action),
            Err(error) => {
                self.body_error(error);
                false
            }
        }
    }

    fn confirm_body_preview(&mut self) -> bool {
        let Some(preview) = self.body_editor.preview.take() else {
            return false;
        };
        match self.document.commit_verified_proposal(&preview.proposal) {
            Ok(_) => {
                self.body_editor.body_name.clear();
                self.body_editor.feature_name.clear();
                self.reconcile_selection();
                self.digest = self.catalog.format(
                    "body-committed",
                    &BTreeMap::from([("action", preview.action)]),
                );
                true
            }
            Err(error) => {
                self.body_error(error);
                false
            }
        }
    }

    fn cancel_body_preview(&mut self) {
        self.body_editor.preview = None;
        self.digest = self.catalog.text("body-cancelled");
    }

    pub(super) fn show_body_editor(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.catalog.text("body-title"))
            .id_salt("body-editor")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("body-editor-scroll")
                    .max_height(700.0)
                    .show(ui, |ui| self.show_body_editor_content(ui));
            });
        ui.separator();
    }

    fn show_body_editor_content(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        self.reconcile_body_editor(&snapshot);
        if let Some(preview) = self.body_editor.preview.as_ref() {
            ui.label(self.catalog.format(
                "body-preview-observational",
                &BTreeMap::from([("action", preview.action.clone())]),
            ));
            let confirm = ui
                .button(self.catalog.text("body-confirm-preview"))
                .clicked();
            let cancel = ui
                .button(self.catalog.text("body-cancel-preview"))
                .clicked();
            if confirm {
                self.confirm_body_preview();
            } else if cancel {
                self.cancel_body_preview();
            }
            return;
        }

        let definitions = snapshot
            .definitions()
            .map(|definition| (definition.id(), definition.name().to_owned()))
            .collect::<Vec<_>>();
        if let Some(mut selected) = self.body_editor.definition {
            let label = self.catalog.text("body-definition");
            let selected_name = definitions
                .iter()
                .find(|(id, _)| *id == selected)
                .map_or_else(|| selected.0.to_string(), |(_, name)| name.clone());
            let response = egui::ComboBox::from_id_salt("body-definition")
                .width(ui.available_width())
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (id, name) in &definitions {
                        ui.selectable_value(&mut selected, *id, name);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
            if self.body_editor.definition != Some(selected) {
                self.body_editor.definition = Some(selected);
                self.body_editor.selected_body = None;
                self.body_editor.source_feature = None;
                self.body_editor.target_body = None;
                self.body_editor.tool_body = None;
                self.body_editor.body_name.clear();
                self.body_editor.feature_name.clear();
                return;
            }
        }

        let Some(definition_id) = self.body_editor.definition else {
            return;
        };
        let Some(definition) = snapshot.definition(definition_id) else {
            return;
        };
        let mut action = None;
        ui.label(self.catalog.text("body-list"));
        for body in definition.bodies() {
            let id = body.id();
            let active = id == definition.active_body_id();
            let selected = self.body_editor.selected_body == Some(id);
            let state = if let Some(feature_id) = body.consumed_by() {
                self.catalog.format(
                    "body-state-consumed",
                    &BTreeMap::from([("feature", feature_id.0.to_string())]),
                )
            } else if active {
                self.catalog.text("body-state-active")
            } else {
                self.catalog.text("body-state-inactive")
            };
            ui.label(self.catalog.format(
                "body-row",
                &BTreeMap::from([
                    ("name", body.name().to_owned()),
                    ("id", id.0.to_string()),
                    ("state", state),
                    (
                        "visibility",
                        self.catalog.text(if body.visible() {
                            "visibility-shown"
                        } else {
                            "visibility-hidden"
                        }),
                    ),
                ]),
            ));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(
                        selected,
                        self.catalog.format(
                            "body-select",
                            &BTreeMap::from([("name", body.name().to_owned())]),
                        ),
                    )
                    .clicked()
                {
                    action = Some(BodyUiAction::Select(id));
                }
                if ui
                    .add_enabled(
                        !active && body.consumed_by().is_none(),
                        egui::Button::new(self.catalog.format(
                            "body-preview-activate",
                            &BTreeMap::from([("name", body.name().to_owned())]),
                        )),
                    )
                    .clicked()
                {
                    action = Some(BodyUiAction::Activate(id));
                }
                if ui
                    .add_enabled(
                        body.consumed_by().is_none(),
                        egui::Button::new(self.catalog.format(
                            if body.visible() {
                                "body-preview-hide"
                            } else {
                                "body-preview-show"
                            },
                            &BTreeMap::from([("name", body.name().to_owned())]),
                        )),
                    )
                    .clicked()
                {
                    action = Some(BodyUiAction::Visibility(id, !body.visible()));
                }
            });
        }

        ui.separator();
        ui.label(self.catalog.text("body-create-title"));
        let body_name_label = self.catalog.text("body-name");
        let body_name = ui.add(
            egui::TextEdit::singleline(&mut self.body_editor.body_name).hint_text(&body_name_label),
        );
        body_name.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &body_name_label)
        });
        let feature_name_label = self.catalog.text("body-feature-name");
        let feature_name = ui.add(
            egui::TextEdit::singleline(&mut self.body_editor.feature_name)
                .hint_text(&feature_name_label),
        );
        feature_name.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &feature_name_label)
        });
        let features = Self::supported_body_features(&snapshot, definition_id);
        if let Some(mut source) = self.body_editor.source_feature {
            let label = self.catalog.text("body-source-feature");
            let selected_name = features
                .iter()
                .find(|(id, _, _)| *id == source)
                .map_or_else(|| source.0.to_string(), |(_, name, _)| name.clone());
            let response = egui::ComboBox::from_id_salt("body-source-feature")
                .width(ui.available_width())
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (id, name, _) in &features {
                        ui.selectable_value(&mut source, *id, name);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
            self.body_editor.source_feature = Some(source);
        }
        if ui
            .add_enabled(
                self.body_editor.source_feature.is_some(),
                egui::Button::new(self.catalog.text("body-preview-create")),
            )
            .clicked()
        {
            action = Some(BodyUiAction::Create);
        }

        ui.separator();
        ui.label(self.catalog.text("body-combine-title"));
        let bodies = definition
            .bodies()
            .filter(|body| body.consumed_by().is_none())
            .map(|body| (body.id(), body.name().to_owned()))
            .collect::<Vec<_>>();
        for (salt, label_key, selected) in [
            ("target", "body-target", &mut self.body_editor.target_body),
            ("tool", "body-tool", &mut self.body_editor.tool_body),
        ] {
            let label = self.catalog.text(label_key);
            let selected_text = selected
                .and_then(|id| {
                    bodies
                        .iter()
                        .find(|(candidate, _)| *candidate == id)
                        .map(|(_, name)| name.clone())
                })
                .unwrap_or_else(|| self.catalog.text("body-none"));
            let response = egui::ComboBox::from_id_salt(("body-operand", salt))
                .width(ui.available_width())
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (id, name) in &bodies {
                        ui.selectable_value(selected, Some(*id), name);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
        }
        let operation_label = self.catalog.text("body-operation");
        let operation = egui::ComboBox::from_id_salt("body-operation")
            .width(ui.available_width())
            .selected_text(self.catalog.text(self.body_editor.operation.label_key()))
            .show_ui(ui, |ui| {
                for choice in BodyBooleanChoice::ALL {
                    ui.selectable_value(
                        &mut self.body_editor.operation,
                        choice,
                        self.catalog.text(choice.label_key()),
                    );
                }
            });
        operation.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &operation_label)
        });
        ui.checkbox(
            &mut self.body_editor.consume_tool,
            self.catalog.text("body-consume-tool"),
        );
        if ui
            .add_enabled(
                self.body_editor.target_body.is_some()
                    && self.body_editor.tool_body.is_some()
                    && self.body_editor.target_body != self.body_editor.tool_body,
                egui::Button::new(self.catalog.text("body-preview-combine")),
            )
            .clicked()
        {
            action = Some(BodyUiAction::Combine);
        }

        match action {
            Some(BodyUiAction::Select(id)) => {
                self.body_editor.selected_body = Some(id);
                self.body_editor.target_body = Some(id);
                if self.body_editor.tool_body == Some(id) {
                    self.body_editor.tool_body = bodies
                        .iter()
                        .map(|(candidate, _)| *candidate)
                        .find(|candidate| *candidate != id);
                }
                self.digest = self
                    .catalog
                    .format("body-selected", &BTreeMap::from([("id", id.0.to_string())]));
            }
            Some(BodyUiAction::Activate(id)) => {
                let name = definition
                    .body(id)
                    .map_or_else(|| id.0.to_string(), |body| body.name().to_owned());
                self.preview_body_command(
                    CanonicalCommand::SetActiveBody { definition_id, id },
                    self.catalog
                        .format("body-action-activate", &BTreeMap::from([("name", name)])),
                );
            }
            Some(BodyUiAction::Visibility(id, visible)) => {
                let name = definition
                    .body(id)
                    .map_or_else(|| id.0.to_string(), |body| body.name().to_owned());
                self.preview_body_command(
                    CanonicalCommand::SetBodyVisibility {
                        definition_id,
                        id,
                        visible,
                    },
                    self.catalog.format(
                        if visible {
                            "body-action-show"
                        } else {
                            "body-action-hide"
                        },
                        &BTreeMap::from([("name", name)]),
                    ),
                );
            }
            Some(BodyUiAction::Create) => {
                self.preview_new_body_feature();
            }
            Some(BodyUiAction::Combine) => {
                self.preview_multibody_boolean();
            }
            None => {}
        }
    }

    #[must_use]
    pub fn body_preview_pending(&self) -> bool {
        self.body_editor.preview.is_some()
    }

    #[must_use]
    pub fn selected_body_id(&self) -> Option<BodyId> {
        self.body_editor.selected_body
    }
}
