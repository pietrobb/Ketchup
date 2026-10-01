//! The Assistant dock panel: conversation, proposals and their value labels.

use crate::*;

impl KetchupApp {
    pub(crate) fn assistant_proposal_value_label(&self, value: &ProposalValue) -> String {
        match value {
            ProposalValue::Missing => self.catalog.text("assistant-value-missing"),
            ProposalValue::Boolean(value) => self.assistant_bool_label(*value),
            ProposalValue::Dimension(value) => self.catalog.format(
                "assistant-value-dimension",
                &BTreeMap::from([
                    ("source", value.source_token().to_owned()),
                    ("value", value.millimetres().to_string()),
                ]),
            ),
            ProposalValue::ProfilePoints(points) => self.catalog.format(
                "assistant-value-profile-points",
                &BTreeMap::from([(
                    "points",
                    points
                        .iter()
                        .map(|point| format!("{},{}", point[0], point[1]))
                        .collect::<Vec<_>>()
                        .join("; "),
                )]),
            ),
            ProposalValue::Transform(value) => self.catalog.format(
                "assistant-value-transform",
                &BTreeMap::from([("matrix", Self::assistant_transform_matrix_label(value))]),
            ),
            ProposalValue::Tag(Some(id)) => self.catalog.format(
                "assistant-value-tag",
                &BTreeMap::from([("id", id.0.to_string())]),
            ),
            ProposalValue::Tag(None) => self.catalog.text("assistant-value-no-tag"),
            ProposalValue::Definition(id) => self.catalog.format(
                "assistant-value-definition",
                &BTreeMap::from([("id", id.0.to_string())]),
            ),
            ProposalValue::Group(Some(id)) => self.catalog.format(
                "assistant-value-group",
                &BTreeMap::from([("id", id.0.to_string())]),
            ),
            ProposalValue::Group(None) => self.catalog.text("assistant-value-no-group"),
            ProposalValue::Occurrences(ids) => self.catalog.format(
                "assistant-value-occurrences",
                &BTreeMap::from([(
                    "ids",
                    ids.iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                )]),
            ),
            ProposalValue::Text(value) => value.clone(),
            ProposalValue::Digest(value) => self.catalog.format(
                "assistant-value-digest",
                &BTreeMap::from([("digest", value.clone())]),
            ),
            ProposalValue::EvaluatorInputState {
                name,
                dimension,
                dependencies,
            } => self.catalog.format(
                "assistant-value-evaluator-input-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("source", dimension.source_token().to_owned()),
                    ("value", dimension.millimetres().to_string()),
                    (
                        "dependencies",
                        comma_list(dependencies.iter().map(|id| id.0)),
                    ),
                ]),
            ),
            ProposalValue::EvaluatorExpressionState {
                name,
                expression,
                dependencies,
            } => self.catalog.format(
                "assistant-value-evaluator-expression-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("expression", expression.clone()),
                    (
                        "dependencies",
                        comma_list(dependencies.iter().map(|id| id.0)),
                    ),
                ]),
            ),
            ProposalValue::EvaluatorRuleState {
                name,
                expression,
                dependencies,
                input_ports,
                output_ports,
                outputs,
                override_parameters,
            } => self.catalog.format(
                "assistant-value-evaluator-rule-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("expression", expression.clone()),
                    (
                        "dependencies",
                        comma_list(dependencies.iter().map(|id| id.0)),
                    ),
                    (
                        "inputs",
                        comma_list(input_ports.iter().map(|port| port.name())),
                    ),
                    (
                        "output-ports",
                        comma_list(output_ports.iter().map(|port| port.name())),
                    ),
                    ("outputs", Self::assistant_rule_outputs_label(outputs)),
                    (
                        "overrides",
                        comma_list(override_parameters.iter().map(|parameter| parameter.name())),
                    ),
                ]),
            ),
            ProposalValue::RuleOverrideState {
                target,
                parameter,
                value,
                health,
            } => {
                let health = match health {
                    ketchup_model::graph::SlotResolution::Resolved => {
                        self.catalog.text("assistant-health-resolved")
                    }
                    ketchup_model::graph::SlotResolution::Ambiguous { segment_index } => {
                        self.catalog.format(
                            "assistant-health-ambiguous",
                            &BTreeMap::from([("segment", segment_index.to_string())]),
                        )
                    }
                    ketchup_model::graph::SlotResolution::Lost { segment_index } => {
                        self.catalog.format(
                            "assistant-health-lost",
                            &BTreeMap::from([("segment", segment_index.to_string())]),
                        )
                    }
                };
                self.catalog.format(
                    "assistant-value-rule-override-state",
                    &BTreeMap::from([
                        ("rule", target.root_rule_node_id.0.to_string()),
                        ("path", Self::assistant_derived_identity_label(target)),
                        ("parameter", parameter.clone()),
                        ("value", value.to_string()),
                        ("health", health),
                    ]),
                )
            }
            ProposalValue::FeatureParameterBindingState {
                target,
                derived_from,
            } => self.catalog.format(
                "assistant-value-feature-parameter-binding-state",
                &BTreeMap::from([
                    ("feature", target.feature_id.0.to_string()),
                    ("slot", target.path.as_str().to_owned()),
                    ("rule", derived_from.root_rule_node_id.0.to_string()),
                    ("path", Self::assistant_derived_identity_label(derived_from)),
                ]),
            ),
            ProposalValue::JointState {
                participant_a,
                participant_b,
                volume_min,
                volume_max,
            } => self.catalog.format(
                "assistant-value-joint-state",
                &BTreeMap::from([
                    (
                        "participant-a",
                        Self::assistant_derived_identity_label(participant_a),
                    ),
                    (
                        "participant-b",
                        Self::assistant_derived_identity_label(participant_b),
                    ),
                    ("min", point_label(volume_min)),
                    ("max", point_label(volume_max)),
                ]),
            ),
            ProposalValue::SpaceState {
                purpose,
                volume_min,
                volume_max,
                adjacent_to,
                accessible_to,
            } => self.catalog.format(
                "assistant-value-space-state",
                &BTreeMap::from([
                    ("purpose", purpose.clone()),
                    ("min", point_label(volume_min)),
                    ("max", point_label(volume_max)),
                    ("adjacent", comma_list(adjacent_to.iter().map(|id| id.0))),
                    (
                        "accessible",
                        comma_list(accessible_to.iter().map(|id| id.0)),
                    ),
                ]),
            ),
            ProposalValue::ClearanceVolumeState {
                owner,
                reason,
                volume_min,
                volume_max,
                coordinate_frame: _,
                tolerance_mm,
                severity,
                derived_from,
            } => {
                let owner = Self::assistant_clearance_owner_label(owner);
                let severity = match severity {
                    ClearanceSeverity::Advisory => "assistant-severity-advisory",
                    ClearanceSeverity::Required => "assistant-severity-required",
                };
                self.catalog.format(
                    "assistant-value-clearance-volume-state",
                    &BTreeMap::from([
                        ("reason", reason.clone()),
                        ("owner", owner),
                        ("min", point_label(volume_min)),
                        ("max", point_label(volume_max)),
                        ("frame", self.catalog.text("assistant-frame-world")),
                        ("tolerance", tolerance_mm.to_string()),
                        ("severity", self.catalog.text(severity)),
                        (
                            "derived",
                            derived_from.as_ref().map_or_else(
                                || self.catalog.text("assistant-value-missing"),
                                Self::assistant_derived_identity_label,
                            ),
                        ),
                    ]),
                )
            }
            ProposalValue::PersistentDimensionState {
                name,
                target,
                presentation,
            } => {
                let target = Self::assistant_dimension_target_label(target);
                self.catalog.format(
                    "assistant-value-persistent-dimension-state",
                    &BTreeMap::from([
                        ("name", name.clone()),
                        ("target", target),
                        ("unit", presentation.unit.label().to_owned()),
                        ("precision", presentation.decimal_places.to_string()),
                    ]),
                )
            }
            ProposalValue::RuleOutputs(outputs) => self.catalog.format(
                "assistant-value-rule-outputs",
                &BTreeMap::from([("outputs", Self::assistant_rule_outputs_label(outputs))]),
            ),
            ProposalValue::TagState { name, visible } => self.catalog.format(
                "assistant-value-tag-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("visible", self.assistant_bool_label(*visible)),
                ]),
            ),
            ProposalValue::CollectionState {
                name,
                occurrence_ids,
            } => self.catalog.format(
                "assistant-value-collection-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("ids", comma_list(occurrence_ids.iter().map(|id| id.0))),
                ]),
            ),
            ProposalValue::DefinitionState {
                name,
                feature_ids,
                local_occurrence_ids,
                local_group_ids,
            } => self.catalog.format(
                "assistant-value-definition-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("features", comma_list(feature_ids.iter().map(|id| id.0))),
                    (
                        "occurrences",
                        comma_list(local_occurrence_ids.iter().map(|id| id.0)),
                    ),
                    ("groups", comma_list(local_group_ids.iter().map(|id| id.0))),
                ]),
            ),
            ProposalValue::DefinitionFeatures(ids) => self.catalog.format(
                "assistant-value-definition-features",
                &BTreeMap::from([(
                    "ids",
                    ids.iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                )]),
            ),
            ProposalValue::ProfileFeatureState {
                definition,
                name,
                points_mm,
            } => self.catalog.format(
                "assistant-value-profile-feature-state",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("definition", definition.0.to_string()),
                    (
                        "points",
                        points_mm
                            .iter()
                            .map(|point| format!("{},{}", point[0], point[1]))
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                ]),
            ),
            ProposalValue::GroupState {
                name,
                transform,
                parent,
            } => {
                let matrix = transform.matrix();
                self.catalog.format(
                    "assistant-value-group-state",
                    &BTreeMap::from([
                        ("name", name.clone()),
                        ("x", matrix[3].to_string()),
                        ("y", matrix[7].to_string()),
                        ("z", matrix[11].to_string()),
                        ("matrix", Self::assistant_transform_matrix_label(transform)),
                        (
                            "parent",
                            self.assistant_optional_id_label(
                                parent.map(|id| id.0),
                                "assistant-value-no-group",
                            ),
                        ),
                    ]),
                )
            }
            ProposalValue::OccurrenceState {
                definition,
                name,
                transform,
                parent,
                tag,
                visible,
            } => {
                let matrix = transform.matrix();
                self.catalog.format(
                    "assistant-value-occurrence-state",
                    &BTreeMap::from([
                        ("name", name.clone()),
                        ("definition", definition.0.to_string()),
                        ("x", matrix[3].to_string()),
                        ("y", matrix[7].to_string()),
                        ("z", matrix[11].to_string()),
                        ("matrix", Self::assistant_transform_matrix_label(transform)),
                        (
                            "parent",
                            self.assistant_optional_id_label(
                                parent.map(|id| id.0),
                                "assistant-value-no-group",
                            ),
                        ),
                        (
                            "tag",
                            self.assistant_optional_id_label(
                                tag.map(|id| id.0),
                                "assistant-value-no-tag",
                            ),
                        ),
                        ("visible", self.assistant_bool_label(*visible)),
                    ]),
                )
            }
        }
    }

    fn assistant_bool_label(&self, value: bool) -> String {
        self.catalog.text(if value {
            "assistant-value-true"
        } else {
            "assistant-value-false"
        })
    }

    /// The id, or the catalog text `missing_key` when there is none.
    fn assistant_optional_id_label(
        &self,
        id: Option<impl std::fmt::Display>,
        missing_key: &str,
    ) -> String {
        id.map_or_else(|| self.catalog.text(missing_key), |id| id.to_string())
    }

    fn assistant_clearance_owner_label(owner: &ketchup_model::space::ClearanceOwner) -> String {
        match owner {
            ketchup_model::space::ClearanceOwner::Occurrence(path) => {
                Self::assistant_instance_path_label(path)
            }
            ketchup_model::space::ClearanceOwner::Space(id) => id.0.to_string(),
        }
    }

    fn assistant_dimension_target_label(
        target: &ketchup_model::document::PersistentDimensionTarget,
    ) -> String {
        match target {
            ketchup_model::document::PersistentDimensionTarget::FeatureParameter(target) => {
                format!("{}:{}", target.feature_id.0, target.path.as_str())
            }
            ketchup_model::document::PersistentDimensionTarget::DerivedOutput(identity) => {
                Self::assistant_derived_identity_label(identity)
            }
            ketchup_model::document::PersistentDimensionTarget::ExactFeatureParameter {
                definition_id,
                producer_feature_id,
                semantic_role,
                source_element_id,
                path,
                value_type,
            } => format!(
                "{}:{}:{}:{}:{}:{value_type:?}",
                definition_id.0,
                producer_feature_id.0,
                semantic_role,
                source_element_id,
                path.as_str()
            ),
        }
    }

    pub(crate) fn show_assistant_inspector(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        ui.checkbox(
            &mut self.assistant.diagnostics_enabled,
            self.catalog.text("assistant-diagnostics-capture"),
        )
        .on_hover_text(self.catalog.text("assistant-diagnostics-capture-help"));
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.assistant.inspector_tab,
                AssistantInspectorTab::ApiLogs,
                self.catalog.text("assistant-diagnostics-api-logs"),
            );
            ui.selectable_value(
                &mut self.assistant.inspector_tab,
                AssistantInspectorTab::Memory,
                self.catalog.text("assistant-diagnostics-memory"),
            );
        });
        ui.separator();
        match self.assistant.inspector_tab {
            AssistantInspectorTab::ApiLogs => {
                ui.small(self.catalog.format(
                    "assistant-diagnostics-log-count",
                    &BTreeMap::from([("count", self.assistant.api_logs.len().to_string())]),
                ));
                if self.assistant.api_logs.is_empty() {
                    ui.weak(self.catalog.text(if self.assistant.diagnostics_enabled {
                        "assistant-diagnostics-no-logs"
                    } else {
                        "assistant-diagnostics-disabled"
                    }));
                    return;
                }
                egui::ScrollArea::horizontal()
                    .id_salt("assistant-api-log-list")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for index in (0..self.assistant.api_logs.len()).rev() {
                                let entry = &self.assistant.api_logs[index];
                                let selected = self.assistant.selected_api_log == Some(index);
                                if ui
                                    .selectable_label(
                                        selected,
                                        format!(
                                            "{} · {}",
                                            entry.request_id, entry.diagnostics.model
                                        ),
                                    )
                                    .clicked()
                                {
                                    self.assistant.selected_api_log = Some(index);
                                }
                            }
                        });
                    });
                let Some(entry) = self
                    .assistant
                    .selected_api_log
                    .and_then(|index| self.assistant.api_logs.get(index))
                    .cloned()
                else {
                    return;
                };
                let diagnostics = entry.diagnostics;
                ui.label(
                    egui::RichText::new(self.catalog.format(
                        "assistant-diagnostics-token-summary",
                        &BTreeMap::from([
                            ("input", diagnostics.input_tokens.to_string()),
                            ("output", diagnostics.output_tokens.to_string()),
                            ("cache", diagnostics.cache_read_tokens.to_string()),
                            ("total", diagnostics.total_tokens().to_string()),
                            (
                                "duration",
                                format!("{:.2}", diagnostics.duration_ms as f64 / 1000.0),
                            ),
                        ]),
                    ))
                    .strong()
                    .color(palette.accent),
                );
                ui.small(self.catalog.format(
                    "assistant-diagnostics-call-meta",
                    &BTreeMap::from([
                        ("provider", diagnostics.provider.clone()),
                        ("model", diagnostics.model.clone()),
                        ("stop", diagnostics.stop_reason.clone()),
                    ]),
                ));
                let payload = serde_json::to_string_pretty(&diagnostics.request_payload)
                    .unwrap_or_else(|_| diagnostics.request_payload.to_string());
                for (key, text) in [
                    ("assistant-diagnostics-request", payload),
                    (
                        "assistant-diagnostics-system-prompt",
                        diagnostics.system_prompt.clone(),
                    ),
                    (
                        "assistant-diagnostics-response",
                        diagnostics.response_text.clone(),
                    ),
                ] {
                    egui::CollapsingHeader::new(self.catalog.text(key))
                        .default_open(key == "assistant-diagnostics-request")
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt(key)
                                .max_height(260.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(text).monospace())
                                            .wrap()
                                            .selectable(true),
                                    );
                                });
                        });
                }
            }
            AssistantInspectorTab::Memory => {
                ui.small(self.catalog.format(
                    "assistant-memory-stored",
                    &BTreeMap::from([("count", self.assistant.memory.entries.len().to_string())]),
                ));
                let search_label = self.catalog.text("assistant-memory-search");
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.assistant.memory_search)
                        .hint_text(&search_label)
                        .desired_width(f32::INFINITY),
                );
                search.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &search_label)
                });
                let matches = self.assistant.memory.search(&self.assistant.memory_search);
                if matches.is_empty() {
                    ui.weak(self.catalog.text("assistant-memory-no-results"));
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("assistant-memory-results")
                    .max_height(360.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        for entry in matches {
                            egui::Frame::new()
                                .fill(palette.panel2)
                                .stroke(Stroke::new(1.0_f32, palette.line))
                                .corner_radius(egui::CornerRadius::same(5))
                                .inner_margin(egui::Margin::same(7))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.strong(format!("#{}", entry.sequence));
                                    ui.label(
                                        egui::RichText::new(&entry.user)
                                            .color(palette.accent)
                                            .strong(),
                                    );
                                    ui.add(
                                        egui::Label::new(&entry.assistant).wrap().selectable(true),
                                    );
                                });
                            ui.add_space(6.0);
                        }
                    });
            }
        }
    }

    pub(crate) fn show_assistant(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        self.show_assistant_header(ui, palette);
        self.show_assistant_messages(ui, palette);
        self.show_assistant_proposal(ui, palette);
        self.show_assistant_verification(ui, palette);
        self.show_assistant_input(ui, palette);

        egui::CollapsingHeader::new(self.catalog.text("assistant-diagnostics-title"))
            .default_open(false)
            .show(ui, |ui| self.show_assistant_inspector(ui));

        self.show_assistant_advanced_tools(ui);
        ui.separator();
    }

    /// Title, conversation document, provider and model choice.
    fn show_assistant_header(&mut self, ui: &mut egui::Ui, palette: Palette) {
        ui.horizontal(|ui| {
            section_header(ui, palette, &self.catalog.text("assistant-title"));
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.catalog.text("assistant-new-chat")).clicked() {
                self.new_assistant_chat();
            }
            let mode_label = match self.assistant.workspace_mode {
                AssistantWorkspaceMode::Dock => self.catalog.text("assistant-open-tab"),
                AssistantWorkspaceMode::Tab => self.catalog.text("assistant-dock-right"),
            };
            if ui.button(mode_label).clicked() {
                self.assistant.workspace_mode = match self.assistant.workspace_mode {
                    AssistantWorkspaceMode::Dock => AssistantWorkspaceMode::Tab,
                    AssistantWorkspaceMode::Tab => AssistantWorkspaceMode::Dock,
                };
            }
        });
        let conversation_document = self
            .file
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(
                || self.catalog.text("document-untitled"),
                |name| name.to_string_lossy().into_owned(),
            );
        ui.label(
            egui::RichText::new(self.catalog.format(
                "assistant-conversation-document",
                &BTreeMap::from([("document", conversation_document)]),
            ))
            .strong()
            .color(palette.text),
        );
        ui.label(
            egui::RichText::new(self.assistant_selection_summary())
                .small()
                .color(palette.accent),
        );
        ui.horizontal_wrapped(|ui| {
            let enabled = self.assembly_kinematic_selection().is_some();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(self.catalog.text("assistant-preview-assembly-joint")),
                )
                .clicked()
            {
                self.prepare_assistant_assembly_joint_from_selection();
            }
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(self.catalog.text("assistant-preview-motion-study")),
                )
                .clicked()
            {
                self.prepare_assistant_motion_study_from_selection();
            }
        });
        ui.label(
            egui::RichText::new(self.catalog.text("assistant-boundary"))
                .small()
                .color(palette.dim),
        );
        let previous_provider = self.assistant.provider;
        ui.label(
            egui::RichText::new(self.catalog.text("assistant-provider"))
                .small()
                .color(palette.dim),
        );
        egui::ComboBox::from_id_salt("assistant-provider")
            .width(ui.available_width())
            .selected_text(self.catalog.text(self.assistant.provider.label_key()))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.assistant.provider,
                    AssistantProvider::AnthropicApi,
                    self.catalog.text("assistant-provider-anthropic-api"),
                );
                ui.selectable_value(
                    &mut self.assistant.provider,
                    AssistantProvider::OpenAiApi,
                    self.catalog.text("assistant-provider-openai-api"),
                );
                #[cfg(feature = "private-oauth")]
                ui.selectable_value(
                    &mut self.assistant.provider,
                    AssistantProvider::ClaudeCodeOauth,
                    self.catalog.text("assistant-provider-claude-oauth"),
                );
                #[cfg(feature = "private-oauth")]
                ui.selectable_value(
                    &mut self.assistant.provider,
                    AssistantProvider::CodexOauth,
                    self.catalog.text("assistant-provider-codex-oauth"),
                );
            });
        if self.assistant.provider != previous_provider {
            self.assistant.model = self.assistant.provider.default_model().to_owned();
        }
        let models = self.assistant_models();
        ui.label(
            egui::RichText::new(self.catalog.text("assistant-model"))
                .small()
                .color(palette.dim),
        );
        egui::ComboBox::from_id_salt("assistant-model")
            .width(ui.available_width())
            .selected_text(&self.assistant.model)
            .show_ui(ui, |ui| {
                for model in models {
                    ui.selectable_value(&mut self.assistant.model, model.clone(), model);
                }
            });
    }

    /// The scrolling conversation.
    fn show_assistant_messages(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let messages_height = if self.assistant.workspace_mode == AssistantWorkspaceMode::Tab {
            (ui.available_height() - 210.0).max(260.0)
        } else {
            (ui.available_height() - 330.0).clamp(220.0, 420.0)
        };
        egui::Frame::new()
            .fill(palette.panel)
            .stroke(Stroke::new(1.0_f32, palette.line))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(messages_height)
                    .show(ui, |ui| {
                        if self.assistant.messages.is_empty() {
                            ui.weak(self.catalog.text("assistant-empty-chat"));
                        }
                        for message in &self.assistant.messages {
                            let (heading, fill, stroke) = match message.role {
                                AssistantMessageRole::User => (
                                    self.catalog.text("assistant-role-you"),
                                    palette.accent_wash(if palette.dark { 42 } else { 28 }),
                                    palette.accent,
                                ),
                                AssistantMessageRole::Assistant => (
                                    self.catalog.text("assistant-role-assistant"),
                                    palette.panel2,
                                    palette.line,
                                ),
                                AssistantMessageRole::Error => (
                                    self.catalog.text("assistant-role-error"),
                                    Color32::from_rgba_unmultiplied(180, 44, 44, 52),
                                    Color32::from_rgb(210, 72, 72),
                                ),
                            };
                            egui::Frame::new()
                                .fill(fill)
                                .stroke(Stroke::new(1.0_f32, stroke))
                                .corner_radius(egui::CornerRadius::same(6))
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(heading);
                                        ui.label(
                                            egui::RichText::new(&message.source)
                                                .small()
                                                .color(palette.dim),
                                        );
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        ui.add(
                                            egui::Label::new(&message.text).wrap().selectable(true),
                                        );
                                        let copy_label =
                                            self.catalog.text("assistant-copy-message");
                                        let copy = ui.add(
                                            egui::Button::new("")
                                                .min_size(Vec2::splat(24.0))
                                                .frame(false),
                                        );
                                        if copy.hovered() {
                                            ui.painter().rect_filled(
                                                copy.rect,
                                                egui::CornerRadius::same(5),
                                                palette.panel2,
                                            );
                                        }
                                        theme::paint_icon(
                                            ui.painter(),
                                            shrink_to_icon(copy.rect, 13.0),
                                            Icon::Copy,
                                            if copy.hovered() {
                                                palette.text
                                            } else {
                                                palette.dim
                                            },
                                            palette.accent,
                                            1.6,
                                        );
                                        name_widget(&copy, true, &copy_label);
                                        if copy.on_hover_text(copy_label).clicked() {
                                            ui.ctx().copy_text(message.text.clone());
                                        }
                                    });
                                });
                            ui.add_space(8.0);
                        }
                        if self.assistant.pending_execution.is_some() {
                            ui.weak(self.catalog.text("assistant-progress-executing"));
                        } else if let Some(task) = self.assistant.chat_task.as_ref() {
                            let elapsed = task.started_at.elapsed();
                            let clock = assistant_clock_frame(elapsed);
                            ui.horizontal(|ui| {
                                ui.weak(clock);
                                ui.weak(self.catalog.text("assistant-progress-requesting"));
                                ui.weak(self.catalog.format(
                                    "assistant-progress-elapsed",
                                    &BTreeMap::from([("time", format_assistant_elapsed(elapsed))]),
                                ));
                            });
                            ui.ctx().request_repaint_after(Duration::from_millis(100));
                        }
                    });
            });
    }

    /// The pending proposal with its review and apply actions.
    fn show_assistant_proposal(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if let Some(proposal) = self.assistant.proposal.clone() {
            let mut confirm_clicked = false;
            let mut cancel_clicked = false;
            let review = egui::Frame::new()
                .fill(palette.panel2)
                .stroke(Stroke::new(1.0_f32, palette.accent))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.strong(self.catalog.text("assistant-review-title"));
                    ui.small(self.catalog.format(
                        "assistant-review-meta",
                        &BTreeMap::from([
                            ("revision", proposal.provenance_revision().to_string()),
                            (
                                "reads",
                                proposal.authoritative_dependencies().len().to_string(),
                            ),
                            ("writes", proposal.authoritative_writes().len().to_string()),
                            ("commands", proposal.cost().commands.to_string()),
                            ("assumptions", proposal.assumptions().len().to_string()),
                        ]),
                    ));
                    ui.small(self.catalog.text("assistant-risk-standard"));
                    ui.weak(self.catalog.text("assistant-review-observational"));
                    if let Some(repair) = &proposal.repair {
                        let before =
                            Self::assistant_validation_issue_total(&repair.validation_before)
                                .unwrap_or_default();
                        let after =
                            Self::assistant_validation_issue_total(&repair.validation_after)
                                .unwrap_or_default();
                        ui.strong(self.catalog.text("assistant-repair-preview-title"));
                        for operation in &repair.program.operations {
                            ui.monospace(self.catalog.format(
                                "assistant-repair-preview-impact",
                                &BTreeMap::from([
                                    ("validator", operation.validator().to_owned()),
                                    ("issue", operation.issue_code().to_owned()),
                                    ("occurrences", format!("{:?}", operation.occurrence_ids())),
                                    ("delta", format!("{:?}", operation.delta_mm())),
                                    ("before", before.to_string()),
                                    ("after", after.to_string()),
                                    (
                                        "requested",
                                        repair.validation_before["requested"].to_string(),
                                    ),
                                ]),
                            ));
                        }
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("assistant-proposal-diff")
                        .max_height(140.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for entry in proposal.authoritative_diff() {
                                ui.label(self.assistant_proposal_diff_label(
                                    &entry.target,
                                    &entry.before,
                                    &entry.after,
                                ));
                            }
                        });
                    ui.horizontal(|ui| {
                        confirm_clicked =
                            ui.button(self.catalog.text("assistant-confirm")).clicked();
                        cancel_clicked = ui.button(self.catalog.text("assistant-cancel")).clicked();
                    });
                });
            // The model stays unchanged until this proposal is answered, so a
            // review that arrives below the fold of the dock would silently
            // stall the whole conversation. Bring a freshly raised proposal
            // into view once, without fighting scrolling afterwards.
            let raised = egui::Id::new("assistant-proposal-raised");
            let fingerprint = proposal.provenance_digest().to_owned();
            if ui.data(|data| data.get_temp::<String>(raised)) != Some(fingerprint.clone()) {
                review.response.scroll_to_me(Some(egui::Align::Center));
                ui.data_mut(|data| data.insert_temp(raised, fingerprint));
            }
            if confirm_clicked {
                self.confirm_assistant_proposal();
            } else if cancel_clicked {
                self.cancel_assistant_proposal();
            }
        }
    }

    /// The verification result of the last applied proposal.
    fn show_assistant_verification(&mut self, ui: &mut egui::Ui, palette: Palette) {
        if let Some(verification) = self.assistant.verification.clone() {
            let can_undo = self.assistant_change_can_undo();
            let mut undo_clicked = false;
            egui::Frame::new()
                .fill(palette.accent_wash(if palette.dark { 32 } else { 20 }))
                .stroke(Stroke::new(1.0_f32, palette.accent))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(self.catalog.text("assistant-result-title"));
                        ui.small(self.catalog.format(
                            "assistant-verification",
                            &BTreeMap::from([
                                ("revision", verification.revision_id.to_string()),
                                ("writes", verification.verified_write_count.to_string()),
                            ]),
                        ));
                    });
                    if let (Some(validator), Some(before), Some(after)) = (
                        &verification.repair_validator,
                        &verification.validation_before,
                        &verification.validation_after,
                    ) {
                        ui.small(
                            self.catalog.format(
                                "assistant-repair-revalidated",
                                &BTreeMap::from([
                                    ("validator", validator.clone()),
                                    ("requested", after["requested"].to_string()),
                                    (
                                        "before",
                                        Self::assistant_validation_issue_total(before)
                                            .unwrap_or_default()
                                            .to_string(),
                                    ),
                                    (
                                        "after",
                                        Self::assistant_validation_issue_total(after)
                                            .unwrap_or_default()
                                            .to_string(),
                                    ),
                                    (
                                        "state",
                                        after["state"]
                                            .as_str()
                                            .unwrap_or("not_evaluated")
                                            .to_owned(),
                                    ),
                                ]),
                            ),
                        );
                    }
                    let undo_label = self.catalog.text("assistant-undo-change");
                    undo_clicked = ui
                        .add_enabled(can_undo, egui::Button::new(&undo_label))
                        .on_hover_text(keymap::shortcut_text(&self.catalog, AppCommand::Undo))
                        .clicked();
                });
            if undo_clicked {
                self.dispatch_command(AppCommand::Undo);
            }
        }
    }

    /// The message input and Send button.
    fn show_assistant_input(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let enter_without_shift =
            ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
        let input_label = self.catalog.text("assistant-input-hint");
        let input = ui.add(
            egui::TextEdit::multiline(&mut self.assistant.input)
                .id_salt("assistant-chat-input")
                .hint_text(&input_label)
                .desired_width(f32::INFINITY)
                .desired_rows(
                    if self.assistant.workspace_mode == AssistantWorkspaceMode::Tab {
                        4
                    } else {
                        3
                    },
                ),
        );
        let send_shortcut = input.has_focus() && enter_without_shift;
        input.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
        });
        input.on_hover_text(self.catalog.text("assistant-send-shortcut"));

        let enabled = self.assistant.chat_task.is_none()
            && self.assistant.pending_execution.is_none()
            && !self.assistant.input.trim().is_empty();
        let send_clicked = ui
            .allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let send_label = self.catalog.text("assistant-send");
                    let send = ui.add_enabled(
                        enabled,
                        egui::Button::new("")
                            .min_size(Vec2::splat(28.0))
                            .fill(if enabled {
                                palette.accent
                            } else {
                                palette.panel2
                            })
                            .stroke(Stroke::NONE)
                            .corner_radius(egui::CornerRadius::same(8)),
                    );
                    theme::paint_icon(
                        ui.painter(),
                        shrink_to_icon(send.rect, 14.0),
                        Icon::Send,
                        if enabled {
                            palette.accent_ink
                        } else {
                            palette.faint
                        },
                        if enabled {
                            palette.accent_ink
                        } else {
                            palette.faint
                        },
                        1.8,
                    );
                    name_widget(&send, enabled, &send_label);
                    send.on_hover_text(self.catalog.text("assistant-send-shortcut"))
                        .clicked()
                },
            )
            .inner;
        if send_clicked || send_shortcut {
            self.send_assistant_message(ui.ctx());
        }
    }

    /// Collapsed advanced tools under the conversation.
    fn show_assistant_advanced_tools(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.catalog.text("assistant-advanced-tools"))
            .default_open(false)
            .show(ui, |ui| {
                egui::ComboBox::from_label(self.catalog.text("assistant-intent"))
                    .selected_text(self.catalog.text(match self.assistant.intent_kind {
                        AssistantIntentKind::CreateEvaluatorInput => {
                            "assistant-intent-create-evaluator-input"
                        }
                        AssistantIntentKind::CreateEvaluatorExpression => {
                            "assistant-intent-create-evaluator-expression"
                        }
                        AssistantIntentKind::CreateEvaluatorRule => {
                            "assistant-intent-create-evaluator-rule"
                        }
                        AssistantIntentKind::CreateRuleOverride => {
                            "assistant-intent-create-rule-override"
                        }
                        AssistantIntentKind::DeleteRuleOverride => {
                            "assistant-intent-delete-rule-override"
                        }
                        AssistantIntentKind::CreateFeatureParameterBinding => {
                            "assistant-intent-create-feature-parameter-binding"
                        }
                        AssistantIntentKind::DeleteFeatureParameterBinding => {
                            "assistant-intent-delete-feature-parameter-binding"
                        }
                        AssistantIntentKind::CreatePersistentDimension => {
                            "assistant-intent-create-persistent-dimension"
                        }
                        AssistantIntentKind::CreateSpace => "assistant-intent-create-space",
                        AssistantIntentKind::CreateClearanceVolume => {
                            "assistant-intent-create-clearance-volume"
                        }
                        AssistantIntentKind::CreateJoint => "assistant-intent-create-joint",
                        AssistantIntentKind::CloneProfileDefinitionAndRepoint => {
                            "assistant-intent-clone-profile-definition"
                        }
                        AssistantIntentKind::ConvertEmptyGroupToComponent => {
                            "assistant-intent-convert-empty-group"
                        }
                        AssistantIntentKind::RecomputeFeatureParameter => {
                            "assistant-intent-recompute-feature-parameter"
                        }
                        AssistantIntentKind::DeleteJoint => "assistant-intent-delete-joint",
                        AssistantIntentKind::DeleteSpace => "assistant-intent-delete-space",
                        AssistantIntentKind::DeleteClearanceVolume => {
                            "assistant-intent-delete-clearance-volume"
                        }
                        AssistantIntentKind::DeletePersistentDimension => {
                            "assistant-intent-delete-persistent-dimension"
                        }
                        AssistantIntentKind::RuleDimension => "assistant-intent-rule",
                        AssistantIntentKind::EvaluatorName => "assistant-intent-evaluator-name",
                        AssistantIntentKind::EvaluatorExpression => {
                            "assistant-intent-evaluator-expression"
                        }
                        AssistantIntentKind::RuleOutputs => "assistant-intent-rule-outputs",
                        AssistantIntentKind::FeatureDimension => "assistant-intent-feature",
                        AssistantIntentKind::ProfilePoints => "assistant-intent-profile-points",
                        AssistantIntentKind::DefinitionName => "assistant-intent-definition-name",
                        AssistantIntentKind::OccurrenceVisibility => {
                            "assistant-intent-occurrence-visibility"
                        }
                        AssistantIntentKind::TagVisibility => "assistant-intent-tag-visibility",
                        AssistantIntentKind::OccurrenceTag => "assistant-intent-occurrence-tag",
                        AssistantIntentKind::OccurrenceDefinition => {
                            "assistant-intent-occurrence-definition"
                        }
                        AssistantIntentKind::OccurrenceParent => {
                            "assistant-intent-occurrence-parent"
                        }
                        AssistantIntentKind::OccurrenceTranslation => {
                            "assistant-intent-occurrence-translation"
                        }
                        AssistantIntentKind::GroupTranslation => {
                            "assistant-intent-group-translation"
                        }
                        AssistantIntentKind::GroupParent => "assistant-intent-group-parent",
                        AssistantIntentKind::CollectionOccurrences => {
                            "assistant-intent-collection-occurrences"
                        }
                        AssistantIntentKind::CreateTag => "assistant-intent-create-tag",
                        AssistantIntentKind::DeleteTag => "assistant-intent-delete-tag",
                        AssistantIntentKind::CreateCollection => {
                            "assistant-intent-create-collection"
                        }
                        AssistantIntentKind::DeleteCollection => {
                            "assistant-intent-delete-collection"
                        }
                        AssistantIntentKind::DeleteGroup => "assistant-intent-delete-group",
                        AssistantIntentKind::DeleteOccurrence => {
                            "assistant-intent-delete-occurrence"
                        }
                        AssistantIntentKind::CreateDefinition => {
                            "assistant-intent-create-definition"
                        }
                        AssistantIntentKind::DeleteDefinition => {
                            "assistant-intent-delete-definition"
                        }
                        AssistantIntentKind::CreateProfileFeature => {
                            "assistant-intent-create-profile-feature"
                        }
                        AssistantIntentKind::DeleteProfileFeature => {
                            "assistant-intent-delete-profile-feature"
                        }
                        AssistantIntentKind::CreateGroup => "assistant-intent-create-group",
                        AssistantIntentKind::CreateOccurrence => {
                            "assistant-intent-create-occurrence"
                        }
                    }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateEvaluatorInput,
                            self.catalog.text("assistant-intent-create-evaluator-input"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateEvaluatorExpression,
                            self.catalog
                                .text("assistant-intent-create-evaluator-expression"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateEvaluatorRule,
                            self.catalog.text("assistant-intent-create-evaluator-rule"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateRuleOverride,
                            self.catalog.text("assistant-intent-create-rule-override"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteRuleOverride,
                            self.catalog.text("assistant-intent-delete-rule-override"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateFeatureParameterBinding,
                            self.catalog
                                .text("assistant-intent-create-feature-parameter-binding"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteFeatureParameterBinding,
                            self.catalog
                                .text("assistant-intent-delete-feature-parameter-binding"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreatePersistentDimension,
                            self.catalog
                                .text("assistant-intent-create-persistent-dimension"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateSpace,
                            self.catalog.text("assistant-intent-create-space"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateClearanceVolume,
                            self.catalog
                                .text("assistant-intent-create-clearance-volume"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateJoint,
                            self.catalog.text("assistant-intent-create-joint"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CloneProfileDefinitionAndRepoint,
                            self.catalog
                                .text("assistant-intent-clone-profile-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::ConvertEmptyGroupToComponent,
                            self.catalog.text("assistant-intent-convert-empty-group"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::RecomputeFeatureParameter,
                            self.catalog
                                .text("assistant-intent-recompute-feature-parameter"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteJoint,
                            self.catalog.text("assistant-intent-delete-joint"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteSpace,
                            self.catalog.text("assistant-intent-delete-space"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteClearanceVolume,
                            self.catalog
                                .text("assistant-intent-delete-clearance-volume"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeletePersistentDimension,
                            self.catalog
                                .text("assistant-intent-delete-persistent-dimension"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::RuleDimension,
                            self.catalog.text("assistant-intent-rule"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::EvaluatorName,
                            self.catalog.text("assistant-intent-evaluator-name"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::EvaluatorExpression,
                            self.catalog.text("assistant-intent-evaluator-expression"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::RuleOutputs,
                            self.catalog.text("assistant-intent-rule-outputs"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::FeatureDimension,
                            self.catalog.text("assistant-intent-feature"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::ProfilePoints,
                            self.catalog.text("assistant-intent-profile-points"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DefinitionName,
                            self.catalog.text("assistant-intent-definition-name"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::OccurrenceVisibility,
                            self.catalog.text("assistant-intent-occurrence-visibility"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::TagVisibility,
                            self.catalog.text("assistant-intent-tag-visibility"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::OccurrenceTranslation,
                            self.catalog.text("assistant-intent-occurrence-translation"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::OccurrenceTag,
                            self.catalog.text("assistant-intent-occurrence-tag"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::OccurrenceDefinition,
                            self.catalog.text("assistant-intent-occurrence-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::OccurrenceParent,
                            self.catalog.text("assistant-intent-occurrence-parent"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::GroupTranslation,
                            self.catalog.text("assistant-intent-group-translation"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::GroupParent,
                            self.catalog.text("assistant-intent-group-parent"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CollectionOccurrences,
                            self.catalog.text("assistant-intent-collection-occurrences"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateTag,
                            self.catalog.text("assistant-intent-create-tag"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteTag,
                            self.catalog.text("assistant-intent-delete-tag"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateCollection,
                            self.catalog.text("assistant-intent-create-collection"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteCollection,
                            self.catalog.text("assistant-intent-delete-collection"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteGroup,
                            self.catalog.text("assistant-intent-delete-group"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteOccurrence,
                            self.catalog.text("assistant-intent-delete-occurrence"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateDefinition,
                            self.catalog.text("assistant-intent-create-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteDefinition,
                            self.catalog.text("assistant-intent-delete-definition"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateProfileFeature,
                            self.catalog.text("assistant-intent-create-profile-feature"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::DeleteProfileFeature,
                            self.catalog.text("assistant-intent-delete-profile-feature"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateGroup,
                            self.catalog.text("assistant-intent-create-group"),
                        );
                        ui.selectable_value(
                            &mut self.assistant.intent_kind,
                            AssistantIntentKind::CreateOccurrence,
                            self.catalog.text("assistant-intent-create-occurrence"),
                        );
                    });
                egui::Grid::new("assistant-intent-inputs").show(ui, |ui| {
                    ui.label(self.catalog.text("assistant-target"));
                    ui.text_edit_singleline(&mut self.assistant.target_input);
                    ui.end_row();
                    ui.label(self.catalog.text("assistant-value-label"))
                        .on_hover_text(self.catalog.text("assistant-value"));
                    ui.text_edit_singleline(&mut self.assistant.value_input);
                    ui.end_row();
                });
                if ui.button(self.catalog.text("assistant-preview")).clicked()
                    && self.prepare_assistant_from_inputs()
                    && self
                        .assistant
                        .proposal
                        .as_ref()
                        .is_some_and(|plan| Self::assistant_proposal_is_low_risk(&plan.proposal))
                {
                    self.confirm_assistant_proposal();
                }
            });
    }

    pub(crate) fn show_fea_review_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.fea_review_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut run = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("fea-review-title"))
            .id(egui::Id::new("static-fea-review"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(format!(
                    "Definition {} · Feature {} · Occurrence {}",
                    pending.definition_id.0, pending.feature_id.0, pending.occurrence_id.0
                ));
                ui.horizontal(|ui| {
                    ui.label(self.catalog.text("fea-review-case"));
                    ui.text_edit_singleline(&mut pending.case_id);
                });
                for (label, value) in [
                    ("fea-review-young", &mut pending.youngs_modulus_mpa),
                    ("fea-review-poisson", &mut pending.poisson_ratio),
                    ("fea-review-yield", &mut pending.yield_strength_mpa),
                    (
                        "fea-review-fixed-face",
                        &mut pending.constrained_face_ordinals,
                    ),
                    ("fea-review-loaded-face", &mut pending.loaded_face_ordinal),
                    ("fea-review-traction-x", &mut pending.traction_x_n_per_mm2),
                    ("fea-review-traction-y", &mut pending.traction_y_n_per_mm2),
                    ("fea-review-traction-z", &mut pending.traction_z_n_per_mm2),
                    ("fea-review-coarse", &mut pending.coarse_deflection_mm),
                    ("fea-review-fine", &mut pending.fine_deflection_mm),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(self.catalog.text(label));
                        ui.text_edit_singleline(value);
                    });
                }
                ui.label(self.catalog.text("fea-review-limits"));
                if let Some(review) = &pending.review {
                    ui.separator();
                    ui.label(format!(
                        "{}: {} · {}: {}",
                        self.catalog.text("fea-review-converged"),
                        review.converged,
                        self.catalog.text("fea-review-within-limits"),
                        review.all_levels_within_declared_limits
                    ));
                    for (index, level) in review.levels.iter().enumerate() {
                        ui.label(format!(
                            "L{}: {} nodes · {} tetra · u={:.6} mm · σvm={:.6} MPa · qmin={:.6}",
                            index + 1,
                            level.node_count,
                            level.element_count,
                            level.maximum_displacement_mm,
                            level.maximum_von_mises_stress_mpa,
                            level.minimum_quality
                        ));
                    }
                    ui.monospace(format!("SHA-256 {}", review.review_digest));
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(self.catalog.text("fea-review-run-confirmed"))
                        .clicked()
                    {
                        run = true;
                    }
                    if ui.button(self.catalog.text("dialog-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if run {
            let result = (|| {
                let parse_f64 = |value: &str, name: &str| {
                    value
                        .parse::<f64>()
                        .map_err(|error| format!("{name} must be a finite number: {error}"))
                };
                let youngs_modulus_mpa = parse_f64(&pending.youngs_modulus_mpa, "Young's modulus")?;
                let poisson_ratio = parse_f64(&pending.poisson_ratio, "Poisson ratio")?;
                let yield_strength_mpa = parse_f64(&pending.yield_strength_mpa, "Yield strength")?;
                let constrained_face_ordinals = pending
                    .constrained_face_ordinals
                    .split(',')
                    .map(str::trim)
                    .map(|value| {
                        value.parse::<u32>().map_err(|error| {
                            format!(
                                "Fixed face ordinals must be comma-separated u32 values: {error}"
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let loaded_face_ordinal = pending
                    .loaded_face_ordinal
                    .parse::<u32>()
                    .map_err(|error| format!("Loaded face ordinal must be u32: {error}"))?;
                let traction = [
                    parse_f64(&pending.traction_x_n_per_mm2, "Traction X")?,
                    parse_f64(&pending.traction_y_n_per_mm2, "Traction Y")?,
                    parse_f64(&pending.traction_z_n_per_mm2, "Traction Z")?,
                ];
                let coarse = parse_f64(&pending.coarse_deflection_mm, "Coarse deflection")?;
                let fine = parse_f64(&pending.fine_deflection_mm, "Fine deflection")?;
                let worker_path = self.exact_worker_executable()?;
                self.reviews.fea_reviews.set_worker_path(worker_path);
                self.reviews
                    .fea_reviews
                    .review(
                        &self.document.current(),
                        &FeaStudyRequest {
                            definition_id: pending.definition_id,
                            feature_id: pending.feature_id,
                            setup: ExactFeaSetup {
                                case_id: pending.case_id.clone(),
                                instance_path: InstancePath::root(pending.occurrence_id),
                                material: FeaMaterial {
                                    id: 1,
                                    youngs_modulus_mpa,
                                    poisson_ratio,
                                    yield_strength_mpa: Some(yield_strength_mpa),
                                },
                                constrained_face_ordinals,
                                face_tractions: vec![ExactFeaFaceTraction {
                                    face_ordinal: loaded_face_ordinal,
                                    traction_local_n_per_mm2: traction,
                                }],
                            },
                            mesh_levels: vec![
                                ExactVolumeMeshWireOptions {
                                    surface_deflection_mm: coarse,
                                    angular_deflection_rad: coarse,
                                    max_tetrahedra: 512,
                                    max_relative_volume_error: 0.05,
                                    min_tetrahedron_quality: 1.0e-6, // not a tolerance: mesh quality
                                },
                                ExactVolumeMeshWireOptions {
                                    surface_deflection_mm: fine,
                                    angular_deflection_rad: fine,
                                    max_tetrahedra: 1_024,
                                    max_relative_volume_error: 0.02,
                                    min_tetrahedron_quality: 1.0e-6, // not a tolerance: mesh quality
                                },
                            ],
                            solve_settings: FeaSolveSettings::default(),
                        },
                        true,
                        &AtomicBool::new(false),
                    )
                    .map_err(|error| error.to_string())
            })();
            match result {
                Ok(review) => {
                    pending.review = Some(review);
                    self.digest = self.catalog.text("fea-review-ready");
                }
                Err(error) => self.digest = error,
            }
        }
        if open && !cancel {
            self.reviews.fea_review_dialog = Some(pending);
        }
    }
}

/// Items of a proposal value as one comma-separated label.
fn comma_list<T: ToString>(items: impl IntoIterator<Item = T>) -> String {
    items
        .into_iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A point of a proposal value as `x,y,z`.
fn point_label<T: std::fmt::Display>(point: &[T; 3]) -> String {
    format!("{},{},{}", point[0], point[1], point[2])
}
