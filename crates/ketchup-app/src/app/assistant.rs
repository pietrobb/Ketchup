//! Assistant requests: context, chat polling, proposal preparation, validation and repair.

use crate::*;

impl KetchupApp {
    pub(crate) fn cancel_pending_assistant_work(&mut self) {
        self.assistant.proposal = None;
        self.assistant.pending_execution = None;
        if let Some(task) = self.assistant.chat_task.take() {
            task.cancellation.cancel();
        }
    }

    pub(crate) fn derive_assistant_workflow_proposal(
        &self,
        intent: &WorkflowIntent,
    ) -> AssistantPlanningResult<Proposal> {
        let target = format!("document:{}", self.document.current().document_id().0);
        let proposal = propose_intent(&self.document, IntentRequest::m7a(intent.clone())).map_err(
            |error| match error {
                ketchup_assistant::intent::IntentError::Canonical(error) => {
                    assistant_canonical_rejection(error, "workflow_intent", &target)
                }
                ketchup_assistant::intent::IntentError::Proposal(error) => {
                    assistant_proposal_prepare_rejection(error, "workflow_intent", &target)
                }
                error => assistant_planning_rejection(
                    "planning.intent_capability_denied",
                    "workflow_intent",
                    &target,
                    error.to_string(),
                    "Request an operation granted to the local Assistant capability set.",
                ),
            },
        )?;
        if !matches!(intent, WorkflowIntent::SetFeatureDimension { .. }) {
            return Ok(proposal);
        }
        let context = ProposalContext {
            principal: ProposalPrincipal::LocalAssistant,
            goal: proposal.goal(),
            assumptions: proposal.assumptions().to_vec(),
            risk: proposal.risk(),
            confirmation: proposal.confirmation().clone(),
            requested_budget: proposal.requested_budget(),
        };
        self.prepare_smart_push_pull_proposal_with_context(proposal.batch().clone(), context)
            .map_err(|error| {
                assistant_proposal_prepare_rejection(error, "set_feature_dimension", &target)
            })
    }

    pub fn prepare_assistant_intent(&mut self, intent: WorkflowIntent) -> bool {
        match self.derive_assistant_workflow_proposal(&intent) {
            Ok(proposal) => {
                self.digest = self.catalog.format(
                    "assistant-digest-preview",
                    &BTreeMap::from([
                        (
                            "reads",
                            proposal.authoritative_dependencies().len().to_string(),
                        ),
                        ("writes", proposal.authoritative_writes().len().to_string()),
                    ]),
                );
                self.status_key = "status-preview";
                self.assistant.verification = None;
                self.assistant.proposal = Some(AssistantPreviewPlan {
                    source: AssistantPreviewSource::Workflow(intent),
                    proposal,
                    repair: None,
                });
                true
            }
            Err(error) => {
                self.assistant.proposal = None;
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.failed_invariant)]),
                );
                false
            }
        }
    }

    pub fn apply_assistant_intent(&mut self, intent: WorkflowIntent) -> bool {
        self.prepare_assistant_intent(intent)
            && self
                .assistant
                .proposal
                .as_ref()
                .is_some_and(|plan| Self::assistant_proposal_is_low_risk(&plan.proposal))
            && self.confirm_assistant_proposal()
    }

    pub fn confirm_assistant_proposal(&mut self) -> bool {
        let Some(plan) = self.assistant.proposal.take() else {
            return false;
        };
        let snapshot = self.document.current();
        let rederived = self.derive_assistant_preview_plan(&plan.source);
        if plan.document_id() != snapshot.document_id()
            || plan.provenance_revision() != snapshot.revision_id()
            || plan.provenance_digest() != snapshot.canonical_digest()
            || rederived.as_ref().ok() != Some(&plan)
        {
            let reason = rederived
                .err()
                .map(|diagnostic| diagnostic.failed_invariant)
                .unwrap_or_else(|| self.catalog.text("assistant-error-stale-response"));
            self.status_key = "status-ready";
            self.digest = self.catalog.format(
                "assistant-digest-rejected",
                &BTreeMap::from([("reason", reason)]),
            );
            return false;
        }
        let repair_selection = match &plan.source {
            AssistantPreviewSource::ValidationRepair(selection) => Some(selection.clone()),
            AssistantPreviewSource::Workflow(_)
            | AssistantPreviewSource::Assembly(_)
            | AssistantPreviewSource::Model(_)
            | AssistantPreviewSource::CadEdit(_) => None,
        };
        let repair_preview = plan.repair.clone();
        match self.commit_verified_proposal_with_work_recovery(&plan.proposal) {
            Ok(committed) => {
                let (repair_program, repair_validator, validation_before, validation_after) =
                    repair_preview.map_or((None, None, None, None), |repair| {
                        let snapshot = self.document.current();
                        self.rebind_exact_results(&snapshot);
                        let selection = repair_selection
                            .as_ref()
                            .expect("repair preview carries its validator selection");
                        let revalidated = self.assistant_validation_context(
                            &snapshot,
                            &self.exact.results,
                            selection,
                        );
                        debug_assert_eq!(revalidated, repair.validation_after);
                        let validators = repair
                            .program
                            .operations
                            .iter()
                            .map(AssistantRepairOperation::validator)
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .collect::<Vec<_>>()
                            .join(" + ");
                        (
                            Some(repair.program),
                            Some(validators),
                            Some(repair.validation_before),
                            Some(revalidated),
                        )
                    });
                let verification = AssistantVerification {
                    revision_id: committed.revision().id(),
                    command_digest: committed.command_digest().to_owned(),
                    result_digest: committed.result_digest().to_owned(),
                    canonical_digest: self.document.current().canonical_digest(),
                    verified_write_count: committed.verified_writes().len(),
                    repair_program,
                    repair_validator,
                    validation_before,
                    validation_after,
                };
                self.digest = self.catalog.format(
                    "assistant-digest-committed",
                    &BTreeMap::from([
                        ("revision", verification.revision_id.to_string()),
                        ("writes", verification.verified_write_count.to_string()),
                    ]),
                );
                self.status_key = "status-ready";
                self.assistant.verification = Some(verification);
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    pub fn cancel_assistant_proposal(&mut self) -> bool {
        if self.assistant.proposal.take().is_none() {
            return false;
        }
        self.status_key = "status-ready";
        self.digest = self.catalog.text("assistant-digest-cancelled");
        true
    }

    #[must_use]
    pub fn assistant_proposal(&self) -> Option<&Proposal> {
        self.assistant.proposal.as_ref().map(|plan| &plan.proposal)
    }

    #[must_use]
    pub fn assistant_repair_program(&self) -> Option<&AssistantRepairProgram> {
        self.assistant
            .proposal
            .as_ref()
            .and_then(|plan| plan.repair.as_ref())
            .map(|repair| &repair.program)
    }

    pub(crate) fn assistant_proposal_target_label(
        &self,
        target: &AuthoritativeDependency,
    ) -> String {
        let identified = match target {
            AuthoritativeDependency::EvaluatorNode(id) => {
                Some(("assistant-entity-evaluator", id.0))
            }
            AuthoritativeDependency::Override(id) => Some(("assistant-entity-override", *id)),
            AuthoritativeDependency::Joint(id) => Some(("assistant-entity-joint", id.0)),
            AuthoritativeDependency::Space(id) => Some(("assistant-entity-space", id.0)),
            AuthoritativeDependency::ClearanceVolume(id) => {
                Some(("assistant-entity-clearance", id.0))
            }
            AuthoritativeDependency::CamPlan(id) => Some(("assistant-entity-cam-plan", id.0)),
            AuthoritativeDependency::PinJoint(id) => Some(("assistant-entity-joint", id.0)),
            AuthoritativeDependency::PersistentDimension(id) => {
                Some(("assistant-entity-persistent-dimension", id.0))
            }
            AuthoritativeDependency::ClassificationDimension(_)
            | AuthoritativeDependency::OccurrenceClassification(_, _) => None,
            AuthoritativeDependency::Tag(id) => Some(("assistant-entity-tag", id.0)),
            AuthoritativeDependency::Collection(id) => Some(("assistant-entity-collection", id.0)),
            AuthoritativeDependency::Import(_)
            | AuthoritativeDependency::Tolerance
            | AuthoritativeDependency::FloorHeight
            | AuthoritativeDependency::GroundedInstances
            | AuthoritativeDependency::ContactJoints
            | AuthoritativeDependency::ProductionCodes
            | AuthoritativeDependency::AssemblyRecipe => None,
            AuthoritativeDependency::Definition(id) => Some(("assistant-entity-definition", id.0)),
            AuthoritativeDependency::DefinitionUsers(id) => {
                Some(("assistant-entity-definition-users", id.0))
            }
            AuthoritativeDependency::Feature(id) => Some(("assistant-entity-feature", id.0)),
            AuthoritativeDependency::FeatureUsers(id) => {
                Some(("assistant-entity-feature-users", id.0))
            }
            AuthoritativeDependency::FeatureParameterBindings(id) => {
                Some(("assistant-entity-feature-bindings", id.0))
            }
            AuthoritativeDependency::BodyFeatureSuppression(definition, body) => {
                return self.catalog.format(
                    "assistant-target-body-suppression",
                    &BTreeMap::from([
                        ("definition", definition.0.to_string()),
                        ("body", body.0.to_string()),
                    ]),
                );
            }
            AuthoritativeDependency::Occurrence(id) => Some(("assistant-entity-occurrence", id.0)),
            AuthoritativeDependency::GroundedOccurrence(id) => {
                Some(("assistant-entity-grounded-occurrence", id.0))
            }
            AuthoritativeDependency::AssemblyMate(id) => {
                Some(("assistant-entity-assembly-mate", id.0))
            }
            AuthoritativeDependency::AssemblyJoint(id) => {
                Some(("assistant-entity-assembly-joint", id.0))
            }
            AuthoritativeDependency::AssemblyMotionCoupling(id) => {
                Some(("assistant-entity-assembly-motion-coupling", id.0))
            }
            AuthoritativeDependency::AssemblyMotionStudy(id) => {
                Some(("assistant-entity-assembly-motion-study", id.0))
            }
            AuthoritativeDependency::MechanicalInterface(id) => {
                Some(("assistant-entity-mechanical-interface", id.0))
            }
            AuthoritativeDependency::MechanicalCondition(id) => {
                Some(("assistant-entity-mechanical-condition", id.0))
            }
            AuthoritativeDependency::DrawingSheet(_) | AuthoritativeDependency::SavedView(_) => {
                None
            }
            AuthoritativeDependency::OccurrenceCollections(id) => {
                Some(("assistant-entity-occurrence-collections", id.0))
            }
            AuthoritativeDependency::Group(id) => Some(("assistant-entity-group", id.0)),
            AuthoritativeDependency::GroupChildren(id) => {
                Some(("assistant-entity-group-children", id.0))
            }
            AuthoritativeDependency::GroupSubtree(id) => {
                Some(("assistant-entity-group-subtree", id.0))
            }
            AuthoritativeDependency::FeatureParameterBinding(target) => {
                return self.catalog.format(
                    "assistant-target-feature-parameter",
                    &BTreeMap::from([
                        ("feature", target.feature_id.0.to_string()),
                        ("slot", target.path.as_str().to_owned()),
                    ]),
                );
            }
            AuthoritativeDependency::LocalGroup(key) => {
                return self.catalog.format(
                    "assistant-target-local-group",
                    &BTreeMap::from([
                        ("definition", key.definition_id.0.to_string()),
                        ("local", key.local_id.0.to_string()),
                    ]),
                );
            }
            AuthoritativeDependency::LocalOccurrence(key) => {
                return self.catalog.format(
                    "assistant-target-local-occurrence",
                    &BTreeMap::from([
                        ("definition", key.definition_id.0.to_string()),
                        ("local", key.local_id.0.to_string()),
                    ]),
                );
            }
        };
        identified.map_or_else(
            || self.catalog.text("assistant-target-canonical"),
            |(kind, id)| {
                self.catalog.format(
                    "assistant-target-identified",
                    &BTreeMap::from([("kind", self.catalog.text(kind)), ("id", id.to_string())]),
                )
            },
        )
    }

    pub(crate) fn assistant_derived_identity_label(identity: &DerivedIdentity) -> String {
        let path = identity
            .slot_path
            .segments()
            .iter()
            .map(|segment| {
                format!(
                    "{}:{}:{}",
                    segment.producer_rule_id.0, segment.output_port, segment.semantic_key
                )
            })
            .collect::<Vec<_>>()
            .join(" / ");
        format!("{} / {path}", identity.root_rule_node_id.0)
    }

    pub(crate) fn assistant_rule_outputs_label(outputs: &[RuleOutput]) -> String {
        fn collect(output: &RuleOutput, prefix: &str, labels: &mut Vec<String>) {
            let segment = output.segment();
            let current = format!(
                "{prefix}{}:{}:{}",
                segment.producer_rule_id.0, segment.output_port, segment.semantic_key
            );
            labels.push(current.clone());
            for child in output.children() {
                collect(child, &format!("{current} / "), labels);
            }
        }

        let mut labels = Vec::new();
        for output in outputs {
            collect(output, "", &mut labels);
        }
        labels.join(", ")
    }

    pub(crate) fn assistant_instance_path_label(path: &InstancePath) -> String {
        let mut label = path.root_occurrence().0.to_string();
        for step in path.steps() {
            match step {
                ketchup_model::document::InstancePathStep::Group(id) => {
                    label.push_str(&format!(" / G{}", id.0));
                }
                ketchup_model::document::InstancePathStep::Occurrence(id) => {
                    label.push_str(&format!(" / O{}", id.0));
                }
            }
        }
        label
    }

    pub(crate) fn assistant_instance_path_value(
        snapshot: &Snapshot,
        path: &InstancePath,
    ) -> serde_json::Value {
        let root_occurrence_id = path.root_occurrence();
        let mut owner_definition_id = snapshot
            .occurrence(root_occurrence_id)
            .expect("scene paths have a root occurrence")
            .definition_id();
        let steps = path
            .steps()
            .iter()
            .map(|step| {
                let step_owner_definition_id = owner_definition_id;
                let (kind, local_id) = match step {
                    ketchup_model::document::InstancePathStep::Group(id) => ("group", id.0),
                    ketchup_model::document::InstancePathStep::Occurrence(id) => {
                        owner_definition_id = snapshot
                            .local_occurrence(ketchup_model::document::LocalOccurrenceKey {
                                definition_id: step_owner_definition_id,
                                local_id: *id,
                            })
                            .expect("scene paths have valid local occurrences")
                            .definition_id();
                        ("occurrence", id.0)
                    }
                };
                serde_json::json!({
                    "owner_definition_id": step_owner_definition_id.0,
                    "kind": kind,
                    "local_id": local_id,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "root_occurrence_id": root_occurrence_id.0,
            "steps": steps,
        })
    }

    pub(crate) fn assistant_transform_matrix_label(transform: &Transform) -> String {
        transform
            .matrix()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(crate) fn assistant_proposal_diff_label(
        &self,
        target: &AuthoritativeDependency,
        before: &ProposalValue,
        after: &ProposalValue,
    ) -> String {
        let target = self.assistant_proposal_target_label(target);
        let key = match (before, after) {
            (ProposalValue::Missing, ProposalValue::Digest(_))
            | (ProposalValue::Missing, ProposalValue::Missing) => "assistant-diff-created-opaque",
            (ProposalValue::Missing, _) => "assistant-diff-created",
            (_, ProposalValue::Missing) => "assistant-diff-removed",
            (ProposalValue::Digest(_), _) | (_, ProposalValue::Digest(_)) => {
                "assistant-diff-updated-opaque"
            }
            _ => "assistant-diff-changed",
        };
        self.catalog.format(
            key,
            &BTreeMap::from([
                ("target", target),
                ("before", self.assistant_proposal_value_label(before)),
                ("after", self.assistant_proposal_value_label(after)),
            ]),
        )
    }

    pub(crate) fn assistant_proposal_is_low_risk(proposal: &Proposal) -> bool {
        matches!(
            proposal.goal(),
            ProposalGoal::SetRuleDimension(_)
                | ProposalGoal::SetFeatureDimension(_)
                | ProposalGoal::SetOccurrenceVisibility(_)
                | ProposalGoal::SetTagVisibility(_)
        )
    }

    #[must_use]
    pub const fn assistant_provider(&self) -> AssistantProvider {
        self.assistant.provider
    }

    #[must_use]
    pub fn assistant_model(&self) -> &str {
        &self.assistant.model
    }

    pub fn select_assistant_provider(&mut self, provider: AssistantProvider) {
        self.assistant.provider = provider;
        self.assistant.model = provider.default_model().to_owned();
    }

    pub fn set_assistant_model(&mut self, model: impl Into<String>) {
        self.assistant.model = model.into();
    }

    #[must_use]
    pub const fn assistant_workspace_mode(&self) -> AssistantWorkspaceMode {
        self.assistant.workspace_mode
    }

    pub fn set_assistant_workspace_mode(&mut self, mode: AssistantWorkspaceMode) {
        self.assistant.workspace_mode = mode;
    }

    #[must_use]
    pub fn assistant_messages(&self) -> &[AssistantChatMessage] {
        &self.assistant.messages
    }

    pub fn set_assistant_diagnostics_enabled(&mut self, enabled: bool) {
        self.assistant.diagnostics_enabled = enabled;
    }

    #[must_use]
    pub fn last_assistant_api_diagnostics(&self) -> Option<&AssistantApiDiagnostics> {
        self.assistant
            .api_logs
            .last()
            .map(|entry| &entry.diagnostics)
    }

    pub fn new_assistant_chat(&mut self) {
        self.assistant.input.clear();
        self.assistant.messages.clear();
        self.cancel_pending_assistant_work();
        self.assistant.verification = None;
        self.assistant.request_sequence = self.assistant.request_sequence.saturating_add(1);
        self.store_assistant_conversation();
    }

    pub(crate) fn store_assistant_conversation(&mut self) {
        let conversation = AssistantConversation {
            document_id: self.document.current().document_id().0,
            messages: self.assistant.messages.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&conversation) else {
            return;
        };
        let Ok(entry) = ketchup_model::persistence::ExtensionEntry::new(
            ASSISTANT_CHAT_NAMESPACE,
            ASSISTANT_CHAT_PATH,
            false,
            bytes,
        ) else {
            return;
        };
        self.file.container_data.set_extension(entry);
    }

    pub(crate) fn store_assistant_memory(&mut self) {
        let Ok(bytes) = serde_json::to_vec(&self.assistant.memory) else {
            return;
        };
        if bytes.len() > MAX_ASSISTANT_MEMORY_STORAGE_BYTES {
            return;
        }
        let Ok(entry) = ketchup_model::persistence::ExtensionEntry::new(
            ASSISTANT_CHAT_NAMESPACE,
            ASSISTANT_MEMORY_PATH,
            false,
            bytes,
        ) else {
            return;
        };
        self.file.container_data.set_extension(entry);
    }

    pub(crate) fn remember_latest_assistant_exchange(&mut self, answer: &str) {
        let Some(user) = self
            .assistant
            .messages
            .iter()
            .rev()
            .find(|message| message.role == AssistantMessageRole::User)
            .map(|message| message.text.clone())
        else {
            return;
        };
        self.assistant.memory.remember(&user, answer);
        self.store_assistant_memory();
    }

    pub(crate) fn load_assistant_conversation(&mut self) {
        let document_id = self.document.current().document_id().0;
        self.assistant.messages = self
            .file
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == ASSISTANT_CHAT_NAMESPACE && entry.path() == ASSISTANT_CHAT_PATH
            })
            .and_then(|entry| serde_json::from_slice::<AssistantConversation>(entry.bytes()).ok())
            .filter(|conversation| {
                conversation.document_id == document_id
                    && conversation.messages.iter().all(|message| {
                        message
                            .diagnostic
                            .as_ref()
                            .is_none_or(|diagnostic| diagnostic.validate().is_ok())
                    })
            })
            .map_or_else(Vec::new, |conversation| conversation.messages);
        self.assistant.saved_conversation_digest =
            assistant_conversation_digest(&self.assistant.messages);
    }

    pub(crate) fn load_assistant_memory(&mut self) {
        let document_id = self.document.current().document_id().0;
        self.assistant.memory = self
            .file
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == ASSISTANT_CHAT_NAMESPACE
                    && entry.path() == ASSISTANT_MEMORY_PATH
            })
            .filter(|entry| entry.bytes().len() <= MAX_ASSISTANT_MEMORY_STORAGE_BYTES)
            .and_then(|entry| serde_json::from_slice::<AssistantProjectMemory>(entry.bytes()).ok())
            .filter(|memory| memory.validate(document_id))
            .unwrap_or_else(|| AssistantProjectMemory::empty(document_id));
    }

    #[must_use]
    pub fn assistant_models(&self) -> Vec<String> {
        assistant_models_for(self.assistant.provider)
    }

    pub(crate) fn assistant_source_label(&self) -> String {
        format!(
            "{} · {}",
            self.catalog.text(self.assistant.provider.label_key()),
            self.assistant.model
        )
    }

    #[must_use]
    pub fn assistant_handshake(&self) -> AssistantHandshake {
        let mut capabilities = BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]);
        if self.assistant.diagnostics_enabled {
            capabilities.insert(AssistantCapability::DebugObservability);
        }
        AssistantHandshake {
            protocol_version: PROTOCOL_VERSION,
            distribution: self.assistant.provider.distribution(),
            provider: self.assistant.provider.protocol_name().to_owned(),
            model: self.assistant.model.clone(),
            capabilities,
        }
    }

    pub fn assistant_context(&self) -> serde_json::Value {
        self.assistant_context_for("")
    }

    pub(crate) fn assistant_validation_context(
        &self,
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        selection: &AssistantValidationSelection,
    ) -> serde_json::Value {
        ketchup_application::validation::assistant_validation_context_with_worker(
            snapshot,
            exact_results,
            &ketchup_application::validation::AssistantValidationSelection {
                mode: selection.mode,
                requested: selection.requested.clone(),
                unknown: selection.unknown.clone(),
            },
            &self.file.container_data,
            self.validator_worker_path(),
            Duration::from_secs(30),
        )
    }

    /// World bounds of every visible body, whatever answers for its geometry.
    ///
    /// The assistant reasons about extents, not about which painter owns a
    /// body, so this deliberately does not go through the render proxies: a
    /// cut or revolved body has no box proxy yet still has bounds, and
    /// describing it as absent would make the model plan around a void.
    pub(crate) fn assistant_body_bounds(
        &self,
        snapshot: &Snapshot,
    ) -> BTreeMap<InstancePath, (DefinitionId, [Vec3; 2])> {
        assistant_body_bounds_from_snapshot(snapshot, &self.exact.results)
    }

    pub(crate) fn assistant_occurrence_records(
        &self,
        snapshot: &Snapshot,
    ) -> Vec<(InstancePath, serde_json::Value)> {
        assistant_occurrence_records_from_snapshot(snapshot, &self.exact.results)
    }

    pub(crate) fn assistant_context_for(&self, query: &str) -> serde_json::Value {
        let snapshot = self.document.current();
        let semantic_state = encode_semantic_state(&snapshot);
        let state_view = bounded_assistant_state_view(&semantic_state.agent());
        let (fea_faces_complete, fea_faces) =
            assistant_fea_face_context(&snapshot, &self.exact.topology_results);
        let project_memory = self.assistant.memory.retrieval_context(query);
        let validation_selection = AssistantValidationSelection::parse(query);
        let validation = self.assistant_validation_context(
            &snapshot,
            &self.exact.results,
            &validation_selection,
        );
        let body_bounds = self.assistant_body_bounds(&snapshot);
        let occurrence_records = self.assistant_occurrence_records(&snapshot);
        let occurrence_count = occurrence_records.len();
        let selected_paths = self.selected_instance_paths();
        let occurrences = occurrence_records
            .iter()
            .filter(|(path, _)| selected_paths.contains(path))
            .chain(
                occurrence_records
                    .iter()
                    .filter(|(path, _)| !selected_paths.contains(path)),
            )
            .take(100)
            .map(|(_, record)| record.clone())
            .collect::<Vec<_>>();
        let conversation = self
            .assistant
            .messages
            .iter()
            .rev()
            .take(20)
            .rev()
            .map(|message| {
                serde_json::json!({
                    "role": match message.role {
                        AssistantMessageRole::User => "user",
                        AssistantMessageRole::Assistant => "assistant",
                        AssistantMessageRole::Error => "error",
                    },
                    "text": message.text,
                    "diagnostic": message.diagnostic,
                })
            })
            .collect::<Vec<_>>();
        let selected_instance_paths = self
            .selected_instance_paths()
            .iter()
            .map(Self::assistant_instance_path_label)
            .collect::<Vec<_>>();
        let (selected_occurrence_ids, selection_scope) = match self.selected_root_occurrence_ids() {
            Ok(ids) => (
                ids.into_iter().map(|id| id.0).collect::<Vec<_>>(),
                "root_occurrences",
            ),
            Err(RootOccurrenceSelectionError::Nested { .. }) => {
                (Vec::new(), "nested_instance_paths")
            }
            Err(RootOccurrenceSelectionError::Mixed { .. }) => (Vec::new(), "mixed_instance_paths"),
        };
        let selected_profile_translation_target = self.assistant_profile_translation_target().map(
            |(definition_id, body_id, profile_id, name)| {
                serde_json::json!({
                    "definition_id": definition_id.0,
                    "body_id": body_id.0,
                    "profile_id": profile_id.0,
                    "name": name,
                })
            },
        );
        let selected_parameter_edit_target = self.assistant_parameter_edit_target().map(
            |(definition_id, body_id, target, name, current_value_mm)| {
                let (feature_id, constraint_id, parameter_path) = match target {
                    ketchup_model::feature_history::ExactParameterEditTarget::FeatureDimension(
                        feature_id,
                    ) => (feature_id, None, None),
                    ketchup_model::feature_history::ExactParameterEditTarget::FeatureParameter(
                        target,
                    ) => (target.feature_id, None, Some(target.path.as_str().to_owned())),
                    ketchup_model::feature_history::ExactParameterEditTarget::SketchConstraintDimension {
                        sketch_id,
                        constraint_id,
                    } => (sketch_id, Some(constraint_id.0), None),
                };
                serde_json::json!({
                    "definition_id": definition_id.0,
                    "body_id": body_id.0,
                    "feature_id": feature_id.0,
                    "constraint_id": constraint_id,
                    "parameter_path": parameter_path,
                    "name": name,
                    "current_value_mm": current_value_mm,
                })
            },
        );
        let mut topology_face_references = assistant_topology_references(
            &snapshot,
            &self.exact.topology_results,
            TopologicalElementKind::Face,
        );
        let topology_face_references_complete = topology_face_references.len() <= 64;
        topology_face_references.truncate(64);
        let topology_face_references = topology_face_references
            .into_iter()
            .map(|reference| {
                serde_json::json!({
                    "definition_id": reference.definition_id.0,
                    "target_feature_id": reference.producer_feature_id.0,
                    "reference_id": reference.lineage_digest,
                })
            })
            .collect::<Vec<_>>();
        let mut topology_edge_references = assistant_topology_references(
            &snapshot,
            &self.exact.topology_results,
            TopologicalElementKind::Edge,
        );
        let topology_edge_references_complete = topology_edge_references.len() <= 64;
        topology_edge_references.truncate(64);
        let topology_edge_references = topology_edge_references
            .into_iter()
            .map(|reference| {
                serde_json::json!({
                    "definition_id": reference.definition_id.0,
                    "target_feature_id": reference.producer_feature_id.0,
                    "reference_id": reference.lineage_digest,
                })
            })
            .collect::<Vec<_>>();
        let boxes = body_bounds
            .into_iter()
            .filter(|(path, _)| path.is_root())
            .take(100)
            .map(|(path, (definition_id, [minimum, maximum]))| {
                let size = maximum - minimum;
                serde_json::json!({
                    "occurrence_id": path.root_occurrence().0,
                    "definition_id": definition_id.0,
                    "origin_mm": [minimum.x, minimum.y, minimum.z],
                    "size_mm": [size.x, size.y, size.z],
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "document_id": snapshot.document_id().0,
            "revision": snapshot.revision_id(),
            "canonical_digest": snapshot.canonical_digest(),
            "state_view": state_view,
            "interoperability": assistant_interoperability_context(&snapshot),
            "project_memory": project_memory,
            "validation": validation,
            "selected_occurrence_ids": selected_occurrence_ids,
            "selected_instance_paths": selected_instance_paths,
            "selection_scope": selection_scope,
            "selected_group_id": self.selection.selected_group.map(|id| id.0),
            "selected_profile_translation_target": selected_profile_translation_target,
            "selected_parameter_edit_target": selected_parameter_edit_target,
            "topology_face_references_complete": topology_face_references_complete,
            "topology_face_references": topology_face_references,
            "fea_faces_complete": fea_faces_complete,
            "fea_faces": fea_faces,
            "topology_edge_references_complete": topology_edge_references_complete,
            "topology_edge_references": topology_edge_references,
            "occurrence_count": occurrence_count,
            "occurrences_complete": occurrence_count <= 100,
            "occurrences": occurrences,
            "boxes": boxes,
            "conversation": conversation,
        })
    }

    pub(crate) fn assistant_request_snapshot(&self, query: &str) -> AssistantRequestSnapshot {
        let snapshot = self.document.current();
        let selected_paths = self.selected_instance_paths();
        let (selected_occurrence_ids, selection_scope) = match self.selected_root_occurrence_ids() {
            Ok(ids) => (
                ids.into_iter().map(|id| id.0).collect::<Vec<_>>(),
                "root_occurrences",
            ),
            Err(RootOccurrenceSelectionError::Nested { .. }) => {
                (Vec::new(), "nested_instance_paths")
            }
            Err(RootOccurrenceSelectionError::Mixed { .. }) => (Vec::new(), "mixed_instance_paths"),
        };
        let selected_profile_translation_target = self
            .assistant_profile_translation_target()
            .map(|(definition_id, body_id, profile_id, name)| {
                serde_json::json!({
                    "definition_id": definition_id.0,
                    "body_id": body_id.0,
                    "profile_id": profile_id.0,
                    "name": name,
                })
            })
            .unwrap_or(serde_json::Value::Null);
        let selected_parameter_edit_target = self
            .assistant_parameter_edit_target()
            .map(|(definition_id, body_id, target, name, current_value_mm)| {
                let (feature_id, constraint_id, parameter_path) = match target {
                    ketchup_model::feature_history::ExactParameterEditTarget::FeatureDimension(
                        feature_id,
                    ) => (feature_id, None, None),
                    ketchup_model::feature_history::ExactParameterEditTarget::FeatureParameter(
                        target,
                    ) => (target.feature_id, None, Some(target.path.as_str().to_owned())),
                    ketchup_model::feature_history::ExactParameterEditTarget::SketchConstraintDimension {
                        sketch_id,
                        constraint_id,
                    } => (sketch_id, Some(constraint_id.0), None),
                };
                serde_json::json!({
                    "definition_id": definition_id.0,
                    "body_id": body_id.0,
                    "feature_id": feature_id.0,
                    "constraint_id": constraint_id,
                    "parameter_path": parameter_path,
                    "name": name,
                    "current_value_mm": current_value_mm,
                })
            })
            .unwrap_or(serde_json::Value::Null);
        AssistantRequestSnapshot {
            snapshot,
            exact_results: self.exact.results.clone(),
            topology_results: self.exact.topology_results.clone(),
            container_data: self.file.container_data.clone(),
            worker_path: self.validator_worker_path(),
            query: query.to_owned(),
            project_memory: self.assistant.memory.clone(),
            conversation: self.assistant.messages.clone(),
            selected_paths,
            selected_occurrence_ids,
            selection_scope,
            selected_group_id: self.selection.selected_group.map(|id| id.0),
            selected_profile_translation_target,
            selected_parameter_edit_target,
            #[cfg(feature = "testing")]
            preparation_delay: self.assistant.context_preparation_delay,
        }
    }

    pub(crate) fn localized_assistant_rejection(
        &self,
        diagnostic: &AssistantRejectionDiagnostic,
        replan_will_run: bool,
    ) -> String {
        let mut text = self.catalog.format(
            "assistant-rejection-detail",
            &BTreeMap::from([
                (
                    "phase",
                    self.catalog
                        .text(assistant_rejection_phase_key(diagnostic.phase)),
                ),
                ("code", diagnostic.code.clone()),
                ("operation", diagnostic.operation.clone()),
                ("target", diagnostic.target.clone()),
                ("invariant", diagnostic.failed_invariant.clone()),
                ("repair", diagnostic.repair_hint.clone()),
                (
                    "retryable",
                    self.catalog.text(if diagnostic.retryable {
                        "assistant-value-true"
                    } else {
                        "assistant-value-false"
                    }),
                ),
            ]),
        );
        if replan_will_run {
            text.push('\n');
            text.push_str(&self.catalog.text("assistant-rejection-replan-once"));
        }
        text
    }

    pub(crate) fn record_assistant_rejection(
        &mut self,
        diagnostic: AssistantRejectionDiagnostic,
        replan_will_run: bool,
    ) -> AssistantRejectionDiagnostic {
        self.assistant.proposal = None;
        self.digest = self.catalog.format(
            "assistant-digest-rejected",
            &BTreeMap::from([("reason", diagnostic.failed_invariant.clone())]),
        );
        self.assistant.messages.push(AssistantChatMessage {
            role: AssistantMessageRole::Error,
            text: self.localized_assistant_rejection(&diagnostic, replan_will_run),
            source: self.catalog.text("assistant-role-error"),
            diagnostic: Some(diagnostic.clone()),
        });
        diagnostic
    }

    pub(crate) fn start_assistant_request(
        &mut self,
        context: &egui::Context,
        message: String,
        request_snapshot: AssistantRequestSnapshot,
        source: String,
        replan_attempted: bool,
        replan_diagnostic: Option<AssistantRejectionDiagnostic>,
    ) -> Result<(), Rejection> {
        let handshake = self.assistant_handshake();
        handshake
            .validate()
            .map_err(|error| failed("assistant.handshake", error))?;
        let request_document_id = self.document.current().document_id();
        let request_revision_id = self.document.current().revision_id();
        let request_canonical_digest = self.document.current().canonical_digest();
        self.assistant.request_sequence = self.assistant.request_sequence.saturating_add(1);
        let request_id = format!("chat-{}", self.assistant.request_sequence);
        let task_request_id = request_id.clone();
        let task_message = message.clone();
        let transport = Arc::clone(&self.assistant.transport);
        let repaint = context.clone();
        let cancellation = AssistantCancellation::default();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = request_snapshot
                .build(&worker_cancellation, true)
                .map_err(|error| failed("assistant.context", error))
                .map(|mut document_context| {
                    if let Some(diagnostic) = replan_diagnostic {
                        document_context["assistant_replan"] = serde_json::json!({
                            "attempt": 1,
                            "max_attempts": 1,
                            "diagnostic": diagnostic,
                        });
                        document_context = bounded_assistant_provider_context(document_context);
                    }
                    document_context
                })
                .and_then(|document_context| {
                    transport.chat_with_diagnostics(
                        handshake,
                        &request_id,
                        &message,
                        &document_context,
                        worker_cancellation,
                    )
                })
                .and_then(|response| {
                    response
                        .validate()
                        .map_err(|error| failed("assistant.response", error))?;
                    Ok(response)
                });
            if sender.send(result).is_ok() {
                repaint.request_repaint();
            }
        });
        self.assistant.chat_task = Some(AssistantChatTask {
            receiver,
            request_id: task_request_id,
            message: task_message,
            replan_attempted,
            started_at: Instant::now(),
            cancellation,
            document_id: request_document_id,
            revision_id: request_revision_id,
            canonical_digest: request_canonical_digest,
            selected_occurrence_ids: self
                .selected_occurrence_ids()
                .into_iter()
                .map(|id| id.0)
                .collect(),
            source,
        });
        Ok(())
    }

    pub(crate) fn send_assistant_message(&mut self, context: &egui::Context) {
        if self.assistant.chat_task.is_some() || self.assistant.pending_execution.is_some() {
            return;
        }
        let message = self.assistant.input.trim().to_owned();
        if message.is_empty() {
            return;
        }
        if assistant_query_requests_repair(&message) {
            self.assistant.input.clear();
            let source = self.assistant_source_label();
            self.assistant.messages.push(AssistantChatMessage {
                role: AssistantMessageRole::User,
                text: message.clone(),
                source: source.clone(),
                diagnostic: None,
            });
            let prepared = self.prepare_assistant_validation_repair(&message);
            self.assistant.messages.push(AssistantChatMessage {
                role: if prepared {
                    AssistantMessageRole::Assistant
                } else {
                    AssistantMessageRole::Error
                },
                text: self.catalog.text(if prepared {
                    "assistant-repair-preview-message"
                } else {
                    "assistant-repair-unavailable"
                }),
                source,
                diagnostic: None,
            });
            self.store_assistant_conversation();
            return;
        }
        self.assistant.input.clear();
        let request_snapshot = self.assistant_request_snapshot(&message);
        let source = self.assistant_source_label();
        self.assistant.messages.push(AssistantChatMessage {
            role: AssistantMessageRole::User,
            text: message.clone(),
            source: source.clone(),
            diagnostic: None,
        });
        if let Err(error) = self.start_assistant_request(
            context,
            message,
            request_snapshot,
            source.clone(),
            false,
            None,
        ) {
            self.assistant.messages.push(AssistantChatMessage {
                role: AssistantMessageRole::Error,
                text: error.reason_text().to_owned(),
                source,
                diagnostic: None,
            });
            self.store_assistant_conversation();
        }
    }

    pub(crate) fn derive_assistant_model_proposal(
        &self,
        intent: &AssistantModelIntent,
    ) -> AssistantPlanningResult<Proposal> {
        let document_target = format!("document:{}", self.document.current().document_id().0);
        intent.validate().map_err(|error| {
            assistant_rejection(
                AssistantRejectionPhase::IntentValidation,
                "intent.model_invalid",
                "model_intent",
                &document_target,
                error.to_string(),
                "Return a bounded model intent that satisfies the Assistant schema invariants.",
                true,
            )
        })?;
        for translation in &intent.translations {
            let occurrence_id = OccurrenceId(translation.occurrence_id);
            if self.document.current().occurrence(occurrence_id).is_none() {
                return Err(assistant_canonical_rejection(
                    CanonicalError::OccurrenceNotFound(occurrence_id),
                    "translate_occurrence",
                    &format!("occurrence:{}", occurrence_id.0),
                ));
            }
        }
        if let [edit] = intent.parameter_edits.as_slice() {
            let target = match edit.constraint_id {
                Some(constraint_id) => ketchup_model::feature_history::ExactParameterEditTarget::SketchConstraintDimension {
                    sketch_id: FeatureId(edit.feature_id),
                    constraint_id: ketchup_geometry::sketch::SketchConstraintId(constraint_id),
                },
                None => ketchup_model::feature_history::ExactParameterEditTarget::FeatureDimension(
                    FeatureId(edit.feature_id),
                ),
            };
            let edit_target = format!("feature:{}", edit.feature_id);
            let (definition_id, body_id, selected_target, _, _) =
                self.assistant_parameter_edit_target().ok_or_else(|| {
                    assistant_planning_rejection(
                        "planning.parameter_target_unavailable",
                        "edit_parameter",
                        &edit_target,
                        "The requested parameter is not the active exact selection.",
                        "Select the exact editable feature or sketch constraint and retry.",
                    )
                })?;
            if definition_id.0 != edit.definition_id
                || body_id.0 != edit.body_id
                || selected_target != target
            {
                return Err(assistant_planning_rejection(
                    "planning.parameter_target_mismatch",
                    "edit_parameter",
                    &edit_target,
                    "The requested parameter does not match the active exact selection.",
                    "Refresh the selection context and retry against the selected parameter.",
                ));
            }
            let dimension =
                Dimension::new(edit.value_mm.to_string(), edit.value_mm).map_err(|error| {
                    assistant_canonical_rejection(error.into(), "edit_parameter", &edit_target)
                })?;
            return ketchup_model::feature_history::prepare_body_parameter_edit(
                &self.document,
                ketchup_model::feature_history::BodyParameterEditRequest {
                    definition_id,
                    body_id,
                    edits: vec![ketchup_model::feature_history::ExactParameterEdit {
                        target,
                        dimension,
                    }],
                },
                ProposalPrincipal::LocalAssistant,
            )
            .map(|preview| preview.proposal)
            .map_err(|error| {
                assistant_feature_edit_rejection(error, "edit_parameter", &edit_target)
            });
        }
        if let [translation] = intent.profile_translations.as_slice() {
            let profile_target = format!("feature:{}", translation.profile_id);
            return ketchup_model::feature_history::prepare_body_profile_translation(
                &self.document,
                ketchup_model::feature_history::BodyProfileTranslationRequest {
                    definition_id: DefinitionId(translation.definition_id),
                    body_id: BodyId(translation.body_id),
                    profile_id: FeatureId(translation.profile_id),
                    delta_mm: translation.delta_mm,
                },
                ProposalPrincipal::LocalAssistant,
            )
            .map(|preview| preview.proposal)
            .map_err(|error| {
                assistant_feature_edit_rejection(error, "translate_profile", &profile_target)
            });
        }
        let batch = self.derive_assistant_model_batch(intent).ok_or_else(|| {
            assistant_planning_rejection(
                "planning.model_intent_unplannable",
                "model_intent",
                &document_target,
                "The validated model intent could not be converted into a canonical command batch.",
                "Refresh document context and request only targets and geometry that still exist.",
            )
        })?;
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
            .map_err(|error| {
                assistant_proposal_prepare_rejection(error, "model_intent", &document_target)
            })
    }

    pub fn plan_assistant_cad_edit_program(
        &self,
        program: &AssistantCadEditProgram,
    ) -> Result<CommandBatch, Box<AssistantRejectionDiagnostic>> {
        ketchup_application::plan_assistant_cad_edit_program(
            &self.document,
            &self.selected_occurrence_ids(),
            &self.exact.topology_results,
            program,
        )
    }

    pub(crate) fn derive_assistant_cad_edit_proposal(
        &self,
        program: &AssistantCadEditProgram,
    ) -> AssistantPlanningResult<Proposal> {
        let target = format!("document:{}", self.document.current().document_id().0);
        let batch = self.plan_assistant_cad_edit_program(program)?;
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
            .map_err(|error| {
                assistant_proposal_prepare_rejection(error, "cad_edit_program", &target)
            })
    }

    pub(crate) fn derive_assistant_model_batch(
        &self,
        intent: &AssistantModelIntent,
    ) -> Option<CommandBatch> {
        let snapshot = self.document.current();
        let mut commands = Vec::new();
        if intent.replace_scene {
            commands.extend(snapshot.collections().map(|collection| {
                CanonicalCommand::SetCollectionOccurrences {
                    id: collection.id(),
                    occurrence_ids: Vec::new(),
                }
            }));
            commands.extend(
                snapshot
                    .occurrences()
                    .map(|item| CanonicalCommand::DeleteOccurrence { id: item.id() }),
            );
            commands.extend(
                snapshot
                    .definitions()
                    .map(|item| CanonicalCommand::DeleteDefinition { id: item.id() }),
            );
        }
        let mut next_definition = snapshot
            .definitions()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let mut next_feature = snapshot
            .features()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let mut next_occurrence = snapshot
            .occurrences()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        for translation in &intent.translations {
            let occurrence = snapshot.occurrence(OccurrenceId(translation.occurrence_id))?;
            let [x, y, z] = translation.delta_mm;
            let transform =
                translated_transform(occurrence.transform(), Vec3::new(x, y, z)).ok()?;
            commands.push(CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(translation.occurrence_id),
                transform,
            });
        }
        for rotation in &intent.rotations {
            let [pivot_x, pivot_y, pivot_z] = rotation.pivot_mm;
            let [axis_x, axis_y, axis_z] = rotation.axis;
            let world_rotation = world_axis_rotation_transform(
                Vec3::new(pivot_x, pivot_y, pivot_z),
                Vec3::new(axis_x, axis_y, axis_z),
                rotation.angle_degrees,
            )
            .ok()?;
            match (rotation.occurrence_id, rotation.group_id) {
                (Some(id), None) => {
                    let occurrence_id = OccurrenceId(id);
                    let occurrence = snapshot.occurrence(occurrence_id)?;
                    let parent_transform = occurrence
                        .parent()
                        .map_or(Some(Transform::identity()), |parent| {
                            snapshot.world_transform_for_group(parent)
                        })?;
                    commands.push(CanonicalCommand::SetOccurrenceTransform {
                        id: occurrence_id,
                        transform: rotation_in_parent_space(
                            world_rotation,
                            parent_transform,
                            occurrence.transform(),
                        )?,
                    });
                }
                (None, Some(id)) => {
                    let group_id = GroupId(id);
                    let group = snapshot.group(group_id)?;
                    let parent_transform = group
                        .parent()
                        .map_or(Some(Transform::identity()), |parent| {
                            snapshot.world_transform_for_group(parent)
                        })?;
                    commands.push(CanonicalCommand::SetGroupTransform {
                        id: group_id,
                        transform: rotation_in_parent_space(
                            world_rotation,
                            parent_transform,
                            group.transform(),
                        )?,
                    });
                }
                _ => return None,
            }
        }
        for array in &intent.linear_arrays {
            let sources = array
                .occurrence_ids
                .iter()
                .map(|id| snapshot.occurrence(OccurrenceId(*id)).cloned())
                .collect::<Option<Vec<_>>>()?;
            for instance in 1..array.instances {
                let [step_x, step_y, step_z] = array.step_mm;
                let delta = Vec3::new(
                    step_x * f64::from(instance),
                    step_y * f64::from(instance),
                    step_z * f64::from(instance),
                );
                for source in &sources {
                    let occurrence = next_occurrence.map(OccurrenceId)?;
                    let transform = translated_transform(source.transform(), delta).ok()?;
                    commands.push(CanonicalCommand::CreateOccurrence {
                        id: occurrence,
                        definition_id: source.definition_id(),
                        name: source.name().to_owned(),
                        transform,
                        parent: source.parent(),
                        tags: source.tags().clone(),
                        visible: source.visible(),
                    });
                    if let Some(color) = source.color() {
                        commands.push(CanonicalCommand::SetOccurrenceColor {
                            id: occurrence,
                            color: Some(color),
                        });
                    }
                    next_occurrence = occurrence.0.checked_add(1);
                }
            }
        }
        for item in &intent.boxes {
            let feature_count;
            let definition = next_definition.map(DefinitionId)?;
            let feature = next_feature.map(FeatureId)?;
            let occurrence = next_occurrence.map(OccurrenceId)?;
            let [width, depth, height] = item.size_mm;
            let [x, y, z] = item.origin_mm;
            let transform = Transform::from_translation(x, y, z).ok()?;
            commands.push(CanonicalCommand::CreateDefinition {
                id: definition,
                name: item.name.clone(),
            });
            if item.subtract_boxes.is_empty() {
                feature_count = 2;
                let extrusion = feature.0.checked_add(1).map(FeatureId)?;
                let height_dimension = Dimension::new(height.to_string(), height).ok()?;
                commands.extend([
                    CanonicalCommand::CreateFeature {
                        id: feature,
                        definition_id: definition,
                        name: format!("{} profile", item.name),
                        kind: FeatureKind::polygon(&[
                            [0.0, 0.0],
                            [width, 0.0],
                            [width, depth],
                            [0.0, depth],
                        ]),
                    },
                    CanonicalCommand::CreateFeature {
                        id: extrusion,
                        definition_id: definition,
                        name: format!("{} extrusion", item.name),
                        kind: FeatureKind::extrusion(feature, height_dimension),
                    },
                ]);
            } else {
                let (feature_commands, count) =
                    assistant_subtracted_box_feature_commands(item, definition, feature)?;
                feature_count = count;
                commands.extend(feature_commands);
            }
            commands.push(CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: item.name.clone(),
                transform,
                parent: None,
                tags: Default::default(),
                visible: true,
            });
            next_definition = definition.0.checked_add(1);
            next_feature = feature.0.checked_add(feature_count);
            next_occurrence = occurrence.0.checked_add(1);
        }
        Some(CommandBatch::new(commands))
    }

    pub(crate) fn assistant_repair_participants(
        &self,
        snapshot: &Snapshot,
    ) -> BTreeMap<OccurrenceId, GeneralBodyParticipant> {
        let exact_results = ExactResultRegistry::carried_forward(snapshot, &self.exact.results);
        snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .take(MAX_ASSISTANT_VALIDATION_OCCURRENCES)
            .filter_map(|occurrence| {
                let occurrence_id = occurrence.instance_path.root_occurrence();
                GeneralBodyParticipant::accept(
                    snapshot,
                    &exact_results,
                    occurrence.instance_path,
                    snapshot.tolerance(),
                )
                .ok()
                .map(|participant| (occurrence_id, participant))
            })
            .collect()
    }

    pub(crate) fn assistant_collision_repair_delta(
        participants: &BTreeMap<OccurrenceId, GeneralBodyParticipant>,
        left_id: OccurrenceId,
        right_id: OccurrenceId,
        tolerance: TolerancePolicy,
    ) -> Option<[f64; 3]> {
        let narrow_phase = general_body_narrow_phase(
            participants.get(&left_id)?,
            participants.get(&right_id)?,
            tolerance,
        )
        .ok()?;
        if narrow_phase.relation != GeneralBodyNarrowPhaseRelation::Intersecting {
            return None;
        }
        let distance_mm = -narrow_phase.signed_separation_mm + tolerance.linear_mm();
        Some(
            narrow_phase
                .separation_axis_world
                .map(|component| component * distance_mm),
        )
    }

    pub(crate) fn assistant_body_projection_interval(
        body: &GeneralBodyParticipant,
        axis: [f64; 3],
    ) -> Option<[f64; 2]> {
        let geometry = body.geometry_evidence();
        let center = geometry.source_frame_center_world_mm();
        let center_projection = (0..3).map(|index| center[index] * axis[index]).sum::<f64>();
        let extents = geometry.source_frame_extents_mm();
        let mut radius = 0.0;
        for (source_axis, extent) in extents.into_iter().enumerate() {
            let direction = geometry.source_axis_world_direction(source_axis)?;
            let scale = geometry.source_axis_world_scale(source_axis)?;
            let alignment = (0..3)
                .map(|index| direction[index] * axis[index])
                .sum::<f64>()
                .abs();
            radius += extent * scale * alignment * 0.5;
        }
        Some([center_projection - radius, center_projection + radius])
    }

    pub(crate) fn assistant_validation_issue_total(validation: &serde_json::Value) -> Option<u64> {
        Some(
            validation["collision"]["issue_count"].as_u64()?
                + validation["gravity_support"]["unsupported_count"].as_u64()?,
        )
    }

    pub(crate) fn assistant_repair_issue_remains(
        validation: &serde_json::Value,
        operation: &AssistantRepairOperation,
    ) -> bool {
        let issues = validation[operation.validator()]["issues"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        match operation {
            AssistantRepairOperation::ResolveCollision {
                left_occurrence_id,
                moved_occurrence_id,
                ..
            } => issues.iter().any(|issue| {
                let found_left = issue["left_occurrence_id"].as_u64();
                let found_right = issue["right_occurrence_id"].as_u64();
                (found_left == Some(*left_occurrence_id)
                    && found_right == Some(*moved_occurrence_id))
                    || (found_left == Some(*moved_occurrence_id)
                        && found_right == Some(*left_occurrence_id))
            }),
            AssistantRepairOperation::RestoreGravitySupport { occurrence_id, .. } => issues
                .iter()
                .any(|issue| issue["occurrence_id"].as_u64() == Some(*occurrence_id)),
        }
    }

    pub(crate) fn assistant_validation_repair_operations(
        &self,
        snapshot: &Snapshot,
        validation: &serde_json::Value,
    ) -> Vec<AssistantRepairOperation> {
        let participants = self.assistant_repair_participants(snapshot);
        let mut operations = Vec::new();
        for issue in validation["collision"]["issues"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let (Some(left_id), Some(right_id)) = (
                issue["left_occurrence_id"].as_u64(),
                issue["right_occurrence_id"].as_u64(),
            ) else {
                continue;
            };
            let (left_id, right_id) = (OccurrenceId(left_id), OccurrenceId(right_id));
            if let Some(delta_mm) = Self::assistant_collision_repair_delta(
                &participants,
                left_id,
                right_id,
                snapshot.tolerance(),
            ) {
                operations.push(AssistantRepairOperation::ResolveCollision {
                    left_occurrence_id: left_id.0,
                    moved_occurrence_id: right_id.0,
                    delta_mm,
                });
            }
        }
        let Ok(roles) = ValidatorRoleIndex::from_snapshot(snapshot) else {
            return operations;
        };
        let Ok(gravity) = assistant_gravity_input(snapshot) else {
            return operations;
        };
        let support_direction = gravity.direction.map(|component| -component);
        for issue in validation["gravity_support"]["issues"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(occurrence_id) = issue["occurrence_id"].as_u64().map(OccurrenceId) else {
                continue;
            };
            let Some(candidate) = participants.get(&occurrence_id) else {
                continue;
            };
            let Some(candidate_role) = roles
                .role(occurrence_id)
                .and_then(|role| PartRole::parse(role.as_str()))
            else {
                continue;
            };
            let Some([candidate_min, _]) =
                Self::assistant_body_projection_interval(candidate, support_direction)
            else {
                continue;
            };
            for (support_occurrence_id, support) in &participants {
                let Some(support_role) = roles
                    .role(*support_occurrence_id)
                    .and_then(|role| PartRole::parse(role.as_str()))
                else {
                    continue;
                };
                if *support_occurrence_id == occurrence_id
                    || support_role.group != candidate_role.group
                    || (support_role.function != RoleFunction::GravityGround
                        && !snapshot.occurrence_is_grounded(*support_occurrence_id))
                {
                    continue;
                }
                let Some([_, support_max]) =
                    Self::assistant_body_projection_interval(support, support_direction)
                else {
                    continue;
                };
                let distance_mm = support_max - candidate_min;
                let delta_mm = support_direction.map(|component| component * distance_mm);
                let operation = AssistantRepairOperation::RestoreGravitySupport {
                    occurrence_id: occurrence_id.0,
                    support_occurrence_id: support_occurrence_id.0,
                    gravity_direction: gravity.direction,
                    delta_mm,
                };
                if operation.validate() {
                    operations.push(operation);
                }
            }
        }
        operations
    }

    pub(crate) fn derive_assistant_validation_repair_plan(
        &self,
        selection: &AssistantValidationSelection,
    ) -> AssistantPlanningResult<AssistantPreviewPlan> {
        let target = format!("document:{}", self.document.current().document_id().0);
        if !selection.is_valid() {
            return Err(assistant_rejection(
                AssistantRejectionPhase::DomainValidation,
                "validator.selection_invalid",
                "validation_repair",
                &target,
                "The requested validator selection is empty or contains unknown validators.",
                "Choose one or more known validators and retry.",
                true,
            ));
        }
        let snapshot = self.document.current();
        let validation_before =
            self.assistant_validation_context(&snapshot, &self.exact.results, selection);
        if validation_before["complete"].as_bool() != Some(true) {
            return Err(assistant_rejection(
                AssistantRejectionPhase::DomainValidation,
                "validator.evidence_incomplete",
                "validation_repair",
                &target,
                "The requested validator evidence is incomplete for the current snapshot.",
                "Resolve missing exact or validator evidence before requesting a repair.",
                true,
            ));
        }

        let mut candidate = snapshot;
        let mut validation_after = validation_before.clone();
        let mut issue_total = Self::assistant_validation_issue_total(&validation_after)
            .ok_or_else(|| {
                assistant_rejection(
                    AssistantRejectionPhase::DomainValidation,
                    "validator.result_invalid",
                    "validation_repair",
                    &target,
                    "The validator result does not contain a bounded issue count.",
                    "Re-run validation against the current canonical snapshot.",
                    true,
                )
            })?;
        let mut target_transforms = BTreeMap::new();
        let mut accepted_operations = Vec::new();
        let mut final_proposal = None;
        let mut last_rejection = None;
        for _ in 0..MAX_ASSISTANT_VALIDATION_ISSUES {
            let mut accepted = None;
            for operation in
                self.assistant_validation_repair_operations(&candidate, &validation_after)
            {
                if !operation.validate() {
                    continue;
                }
                let moved_occurrence = OccurrenceId(operation.moved_occurrence_id());
                let Some(occurrence) = candidate.occurrence(moved_occurrence) else {
                    continue;
                };
                let [x, y, z] = operation.delta_mm();
                let Ok(transform) =
                    translated_transform(occurrence.transform(), Vec3::new(x, y, z))
                else {
                    continue;
                };
                let mut trial_transforms = target_transforms.clone();
                trial_transforms.insert(moved_occurrence, transform);
                let commands = trial_transforms
                    .iter()
                    .map(|(id, transform)| CanonicalCommand::SetOccurrenceTransform {
                        id: *id,
                        transform: *transform,
                    })
                    .collect();
                let proposal = match self.document.prepare_proposal_with_context(
                    CommandBatch::new(commands),
                    ProposalContext::local_assistant_model(),
                ) {
                    Ok(proposal) => proposal,
                    Err(error) => {
                        last_rejection = Some(assistant_proposal_prepare_rejection(
                            error,
                            "validation_repair",
                            &target,
                        ));
                        continue;
                    }
                };
                let trial_candidate = match self.document.preview_batch(proposal.batch()) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        last_rejection = Some(assistant_canonical_rejection(
                            error,
                            "validation_repair",
                            &target,
                        ));
                        continue;
                    }
                };
                let trial_results =
                    ExactResultRegistry::carried_forward(&trial_candidate, &self.exact.results);
                let trial_validation =
                    self.assistant_validation_context(&trial_candidate, &trial_results, selection);
                let Some(trial_total) = Self::assistant_validation_issue_total(&trial_validation)
                else {
                    continue;
                };
                if trial_validation["complete"].as_bool() != Some(true)
                    || trial_validation["requested"] != validation_before["requested"]
                    || Self::assistant_repair_issue_remains(&trial_validation, &operation)
                    || trial_total >= issue_total
                {
                    continue;
                }
                accepted = Some((
                    operation,
                    trial_transforms,
                    proposal,
                    trial_candidate,
                    trial_validation,
                    trial_total,
                ));
                break;
            }
            let Some((
                operation,
                transforms,
                proposal,
                next_candidate,
                next_validation,
                next_total,
            )) = accepted
            else {
                break;
            };
            accepted_operations.push(operation);
            target_transforms = transforms;
            final_proposal = Some(proposal);
            candidate = next_candidate;
            validation_after = next_validation;
            issue_total = next_total;
            if issue_total == 0 {
                break;
            }
        }

        let proposal = final_proposal.ok_or_else(|| {
            last_rejection.unwrap_or_else(|| {
                assistant_rejection(
                    AssistantRejectionPhase::DomainValidation,
                    "validator.no_safe_repair",
                    "validation_repair",
                    &target,
                    "No bounded repair candidate reduced the requested validator issues.",
                    "Revise the model or request a narrower repair after reviewing the validator issues.",
                    true,
                )
            })
        })?;
        let program = AssistantRepairProgram {
            schema: ASSISTANT_REPAIR_PROGRAM_SCHEMA_V1.to_owned(),
            document_id: validation_before["document_id"]
                .as_u64()
                .expect("complete validator evidence carries a document ID"),
            revision_id: validation_before["revision"]
                .as_u64()
                .expect("complete validator evidence carries a revision"),
            canonical_digest: validation_before["canonical_digest"]
                .as_str()
                .expect("complete validator evidence carries a canonical digest")
                .to_owned(),
            max_operations: MAX_ASSISTANT_VALIDATION_ISSUES,
            operations: accepted_operations,
        };
        if !program.validate() {
            return Err(assistant_rejection(
                AssistantRejectionPhase::DomainValidation,
                "validator.repair_program_invalid",
                "validation_repair",
                &target,
                "The generated repair program violates its typed bounds or provenance contract.",
                "Re-run validation against the current canonical snapshot.",
                true,
            ));
        }
        Ok(AssistantPreviewPlan {
            source: AssistantPreviewSource::ValidationRepair(selection.clone()),
            proposal,
            repair: Some(AssistantRepairPreview {
                program,
                validation_before,
                validation_after,
            }),
        })
    }

    pub(crate) fn derive_assistant_preview_plan(
        &self,
        source: &AssistantPreviewSource,
    ) -> AssistantPlanningResult<AssistantPreviewPlan> {
        let proposal = match source {
            AssistantPreviewSource::Workflow(intent) => {
                self.derive_assistant_workflow_proposal(intent)?
            }
            AssistantPreviewSource::Assembly(source) => self
                .derive_assembly_preview_proposal(source)
                .map_err(|error| {
                    assistant_planning_rejection(
                        "planning.assembly_kinematics_invalid",
                        "assembly_kinematics",
                        "current_selection",
                        error.reason_text(),
                        "Restore the original two-occurrence selection and retry the preview.",
                    )
                })?,
            AssistantPreviewSource::Model(intent) => {
                self.derive_assistant_model_proposal(intent)?
            }
            AssistantPreviewSource::CadEdit(program) => {
                self.derive_assistant_cad_edit_proposal(program)?
            }
            AssistantPreviewSource::ValidationRepair(selection) => {
                return self.derive_assistant_validation_repair_plan(selection);
            }
        };
        Ok(AssistantPreviewPlan {
            source: source.clone(),
            proposal,
            repair: None,
        })
    }

    pub(crate) fn prepare_assistant_validation_repair(&mut self, query: &str) -> bool {
        let source =
            AssistantPreviewSource::ValidationRepair(AssistantValidationSelection::parse(query));
        let plan = match self.derive_assistant_preview_plan(&source) {
            Ok(plan) => plan,
            Err(error) => {
                self.assistant.proposal = None;
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.failed_invariant)]),
                );
                return false;
            }
        };
        self.digest = self.catalog.format(
            "assistant-digest-preview",
            &BTreeMap::from([
                ("reads", plan.authoritative_dependencies().len().to_string()),
                ("writes", plan.authoritative_writes().len().to_string()),
            ]),
        );
        self.status_key = "status-preview";
        self.assistant.verification = None;
        self.assistant.proposal = Some(plan);
        true
    }

    pub(crate) fn prepare_assistant_preview_source(
        &mut self,
        source: AssistantPreviewSource,
    ) -> AssistantPlanningResult<()> {
        let plan = self.derive_assistant_preview_plan(&source)?;
        self.digest = self.catalog.format(
            "assistant-digest-preview",
            &BTreeMap::from([
                ("reads", plan.authoritative_dependencies().len().to_string()),
                ("writes", plan.authoritative_writes().len().to_string()),
            ]),
        );
        self.status_key = "status-preview";
        self.assistant.verification = None;
        self.assistant.proposal = Some(plan);
        Ok(())
    }

    pub(crate) fn prepare_assistant_model_intent_result(
        &mut self,
        intent: AssistantModelIntent,
    ) -> AssistantPlanningResult<()> {
        self.prepare_assistant_preview_source(AssistantPreviewSource::Model(intent))
    }

    pub fn prepare_assistant_model_intent(&mut self, intent: AssistantModelIntent) -> bool {
        match self.prepare_assistant_model_intent_result(intent) {
            Ok(()) => true,
            Err(error) => {
                self.assistant.proposal = None;
                self.digest = self.catalog.format(
                    "assistant-digest-rejected",
                    &BTreeMap::from([("reason", error.failed_invariant)]),
                );
                false
            }
        }
    }

    pub fn apply_assistant_model_intent(&mut self, intent: AssistantModelIntent) -> bool {
        self.prepare_assistant_model_intent(intent)
            && self
                .assistant
                .proposal
                .as_ref()
                .is_some_and(|plan| Self::assistant_proposal_is_low_risk(&plan.proposal))
            && self.confirm_assistant_proposal()
    }

    pub(crate) fn poll_assistant_chat(&mut self, context: &egui::Context) {
        if let Some(mut pending) = self.assistant.pending_execution.take() {
            let snapshot = self.document.current();
            if snapshot.document_id() != pending.document_id
                || snapshot.revision_id() != pending.revision_id
                || snapshot.canonical_digest() != pending.canonical_digest
            {
                self.assistant.messages.push(AssistantChatMessage {
                    role: AssistantMessageRole::Error,
                    text: self.catalog.text("assistant-error-stale-response"),
                    source: self.catalog.text("assistant-role-error"),
                    diagnostic: None,
                });
            } else if let Some(program) = pending.cad_edit_program.take() {
                let metadata_only = program.operations.iter().all(|operation| {
                    matches!(
                        operation,
                        AssistantCadEditOperation::SetOccurrenceClassification { .. }
                    )
                });
                if metadata_only {
                    match self
                        .prepare_assistant_preview_source(AssistantPreviewSource::CadEdit(program))
                    {
                        Ok(()) => {
                            let answer = pending.result.message;
                            self.remember_latest_assistant_exchange(&answer);
                            self.assistant.messages.push(AssistantChatMessage {
                                role: AssistantMessageRole::Assistant,
                                text: answer,
                                source: pending.source,
                                diagnostic: None,
                            });
                        }
                        Err(rejection) => {
                            self.record_assistant_rejection(*rejection, false);
                        }
                    }
                } else {
                    match live_bridge::LiveBridge::apply_assistant_cad_program(self, program) {
                        Ok(value) => {
                            let verification = (|| {
                                Some(AssistantVerification {
                                    revision_id: value["after"]["revision"].as_u64()?,
                                    command_digest: value["diff"]["command_digest"]
                                        .as_str()?
                                        .to_owned(),
                                    result_digest: value["diff"]["result_digest"]
                                        .as_str()?
                                        .to_owned(),
                                    canonical_digest: value["after"]["canonical_digest"]
                                        .as_str()?
                                        .to_owned(),
                                    verified_write_count: value["diff"]["entry_count"]
                                        .as_u64()?
                                        .try_into()
                                        .ok()?,
                                    repair_program: None,
                                    repair_validator: None,
                                    validation_before: None,
                                    validation_after: Some(value["validation"].clone()),
                                })
                            })();
                            if let Some(verification) = verification {
                                self.assistant.verification = Some(verification);
                                let answer = pending.result.message;
                                self.remember_latest_assistant_exchange(&answer);
                                self.assistant.messages.push(AssistantChatMessage {
                                    role: AssistantMessageRole::Assistant,
                                    text: answer,
                                    source: pending.source,
                                    diagnostic: None,
                                });
                            } else {
                                self.assistant.messages.push(AssistantChatMessage {
                                    role: AssistantMessageRole::Error,
                                    text: "apply_and_verify: invalid_result".to_owned(),
                                    source: pending.source,
                                    diagnostic: None,
                                });
                            }
                        }
                        Err(
                            live_bridge::PlanRejection::Planning(diagnostic)
                            | live_bridge::PlanRejection::CapabilityGap(diagnostic),
                        ) => {
                            self.record_assistant_rejection(*diagnostic, false);
                        }
                        Err(live_bridge::PlanRejection::Code(code)) => {
                            let details = live_bridge::take_error_details();
                            let detail = |key: &str| {
                                details
                                    .as_ref()
                                    .and_then(|details| details[key].as_str())
                                    .map(str::to_owned)
                            };
                            let mut text = format!("apply_and_verify: {code}");
                            if let Some(message) = detail("reason") {
                                text.push_str(&format!(": {message}"));
                            }
                            if let Some(hint) = detail("fix_hint") {
                                text.push_str(&format!(" ({hint})"));
                            }
                            self.assistant.messages.push(AssistantChatMessage {
                                role: AssistantMessageRole::Error,
                                text,
                                source: pending.source,
                                diagnostic: None,
                            });
                        }
                    }
                }
            } else {
                let source = AssistantPreviewSource::Model(
                    pending
                        .result
                        .model_intent
                        .take()
                        .expect("pending execution always carries one mutation program"),
                );
                match self.prepare_assistant_preview_source(source) {
                    Ok(()) => {
                        let answer = pending.result.message;
                        self.remember_latest_assistant_exchange(&answer);
                        self.assistant.messages.push(AssistantChatMessage {
                            role: AssistantMessageRole::Assistant,
                            text: answer,
                            source: pending.source,
                            diagnostic: None,
                        });
                    }
                    Err(rejection) => {
                        let replan_will_run = rejection.retryable && !pending.replan_attempted;
                        let diagnostic =
                            self.record_assistant_rejection(*rejection, replan_will_run);
                        if replan_will_run {
                            let request_snapshot =
                                self.assistant_request_snapshot(&pending.message);
                            if let Err(refusal) = self.start_assistant_request(
                                context,
                                pending.message,
                                request_snapshot,
                                pending.source.clone(),
                                true,
                                Some(diagnostic),
                            ) {
                                self.assistant.messages.push(AssistantChatMessage {
                                    role: AssistantMessageRole::Error,
                                    text: refusal.reason_text().to_owned(),
                                    source: pending.source,
                                    diagnostic: None,
                                });
                            }
                        }
                    }
                }
            }
            self.store_assistant_conversation();
            return;
        }

        let Some(task) = self.assistant.chat_task.as_ref() else {
            return;
        };
        let source = task.source.clone();
        let request_id = task.request_id.clone();
        let request_message = task.message.clone();
        let replan_attempted = task.replan_attempted;
        let request_document_id = task.document_id;
        let request_revision_id = task.revision_id;
        let request_canonical_digest = task.canonical_digest.clone();
        let request_selected_occurrence_ids = task.selected_occurrence_ids.clone();
        let snapshot = self.document.current();
        if snapshot.document_id() != request_document_id
            || snapshot.revision_id() != request_revision_id
            || snapshot.canonical_digest() != request_canonical_digest
        {
            let task = self
                .assistant
                .chat_task
                .take()
                .expect("stale assistant task is still pending");
            task.cancellation.cancel();
            self.assistant.messages.push(AssistantChatMessage {
                role: AssistantMessageRole::Error,
                text: self.catalog.text("assistant-error-stale-response"),
                source: self.catalog.text("assistant-role-error"),
                diagnostic: None,
            });
            self.store_assistant_conversation();
            return;
        }
        match task.receiver.try_recv() {
            Ok(response) => {
                self.assistant.chat_task = None;
                let result = response.and_then(|response| {
                    if let Some(diagnostics) = response.diagnostics {
                        self.assistant.api_logs.push(AssistantApiLogEntry {
                            request_id,
                            diagnostics,
                        });
                        if self.assistant.api_logs.len() > 100 {
                            self.assistant
                                .api_logs
                                .drain(..self.assistant.api_logs.len() - 100);
                        }
                        self.assistant.selected_api_log =
                            self.assistant.api_logs.len().checked_sub(1);
                    }
                    let mut cad_edit_program = response.cad_edit_program;
                    if let Some(program) = cad_edit_program.as_mut() {
                        bind_assistant_cad_current_selection(
                            program,
                            &request_selected_occurrence_ids,
                        );
                    }
                    ketchup_scheduler::assistant::ensure_single_assistant_action([
                        response.result.model_intent.is_some(),
                        cad_edit_program.is_some(),
                        response.fea_review.is_some(),
                    ])
                    .map_err(|error| failed("assistant.single_action", error))?;
                    Ok((response.result, cad_edit_program, response.fea_review))
                });
                match result {
                    Ok((result, _, Some(request))) => {
                        match self.prepare_assistant_fea_review(&request) {
                            Ok(()) => {
                                self.remember_latest_assistant_exchange(&result.message);
                                self.assistant.messages.push(AssistantChatMessage {
                                    role: AssistantMessageRole::Assistant,
                                    text: result.message,
                                    source,
                                    diagnostic: None,
                                });
                            }
                            Err(diagnostic) => {
                                self.assistant.messages.push(AssistantChatMessage {
                                    role: AssistantMessageRole::Error,
                                    text: self.localized_assistant_rejection(&diagnostic, false),
                                    source,
                                    diagnostic: Some(*diagnostic),
                                });
                            }
                        }
                    }
                    Ok((result, cad_edit_program, None))
                        if result.model_intent.is_some() || cad_edit_program.is_some() =>
                    {
                        self.assistant.pending_execution = Some(AssistantPendingExecution {
                            result,
                            cad_edit_program,
                            message: request_message,
                            replan_attempted,
                            document_id: request_document_id,
                            revision_id: request_revision_id,
                            canonical_digest: request_canonical_digest,
                            source,
                        });
                        context.request_repaint();
                        return;
                    }
                    Ok((result, _, None)) => {
                        self.remember_latest_assistant_exchange(&result.message);
                        self.assistant.messages.push(AssistantChatMessage {
                            role: AssistantMessageRole::Assistant,
                            text: result.message,
                            source,
                            diagnostic: None,
                        });
                    }
                    Err(refusal) => self.assistant.messages.push(AssistantChatMessage {
                        role: AssistantMessageRole::Error,
                        text: refusal.reason_text().to_owned(),
                        source,
                        diagnostic: None,
                    }),
                }
                self.store_assistant_conversation();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.assistant.chat_task = None;
                self.assistant.messages.push(AssistantChatMessage {
                    role: AssistantMessageRole::Error,
                    text: self.catalog.text("assistant-error-disconnected"),
                    source,
                    diagnostic: None,
                });
                self.store_assistant_conversation();
            }
        }
    }

    #[must_use]
    pub const fn assistant_verification(&self) -> Option<&AssistantVerification> {
        self.assistant.verification.as_ref()
    }

    #[must_use]
    pub fn assistant_change_can_undo(&self) -> bool {
        self.assistant
            .verification
            .as_ref()
            .is_some_and(|verification| {
                let snapshot = self.document.current();
                snapshot.revision_id() == verification.revision_id
                    && snapshot.canonical_digest() == verification.canonical_digest
                    && self.can_undo()
            })
    }

    pub(crate) fn assistant_selection_summary(&self) -> String {
        let selected = match self.selected_root_occurrence_ids() {
            Ok(selected) => selected,
            Err(error) => {
                return self
                    .root_occurrence_selection_error(&error)
                    .reason_text()
                    .to_owned();
            }
        };
        match selected.len() {
            0 => self.catalog.text("assistant-selection-none"),
            1 => {
                let id = selected
                    .first()
                    .expect("one selected occurrence must have a first item");
                let name = self
                    .document
                    .current()
                    .scene_query()
                    .into_iter()
                    .find(|occurrence| {
                        occurrence.instance_path.is_root() && occurrence.occurrence_id == *id
                    })
                    .map_or_else(
                        || format!("#{}", id.0),
                        |occurrence| occurrence.occurrence_name,
                    );
                self.catalog
                    .format("assistant-selection-one", &BTreeMap::from([("name", name)]))
            }
            count => self.catalog.format(
                "assistant-selection-many",
                &BTreeMap::from([("count", count.to_string())]),
            ),
        }
    }

    #[cfg(feature = "testing")]
    pub fn headless_set_assistant_context_preparation_delay(&mut self, delay: Duration) {
        self.assistant.context_preparation_delay = delay;
    }

    pub fn prepare_assistant_general_finish(
        &mut self,
        locator: TopologicalPickLocator,
        kind: GeneralFinishKind,
        amount_mm: f64,
    ) -> bool {
        if !amount_mm.is_finite() || amount_mm <= 0.0 || !self.select_topological_locator(locator) {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = match kind {
            GeneralFinishKind::Shell => ActiveTool::Shell,
            GeneralFinishKind::Fillet => ActiveTool::Fillet,
            GeneralFinishKind::Chamfer => ActiveTool::Chamfer,
        };
        self.value_box.input = amount_mm.to_string();
        self.refresh_general_finish_preview()
    }

    pub fn confirm_assistant_general_finish(&mut self) -> bool {
        self.confirm_general_finish_preview()
    }

    pub(crate) fn assistant_feature_parameter_target(
        &self,
        feature_id: FeatureId,
        path: &str,
    ) -> Option<FeatureParameterTarget> {
        let descriptor = self
            .document
            .current()
            .feature(feature_id)?
            .kind()
            .parameter_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.path().as_str() == path)?;
        Some(FeatureParameterTarget {
            feature_id,
            path: descriptor.path().clone(),
            value_type: descriptor.value_type(),
        })
    }

    pub(crate) fn prepare_assistant_from_inputs(&mut self) -> bool {
        let intent = self
            .assistant
            .target_input
            .trim()
            .parse::<u64>()
            .map_or(Err("assistant-error-target"), |target| {
                self.assistant_intent_from_inputs(target, &self.assistant.value_input)
            });
        match intent {
            Ok(intent) => self.prepare_assistant_intent(intent),
            Err(error_key) => {
                self.digest = self.catalog.text(error_key);
                false
            }
        }
    }

    /// The workflow intent the assistant inputs describe, or the catalog key of
    /// the error that tells the user which field to fix.
    fn assistant_intent_from_inputs(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        Ok(match self.assistant.intent_kind {
            AssistantIntentKind::CreateEvaluatorInput => {
                let Some((name, value_text)) = value_text.split_once(':') else {
                    return Err("assistant-error-create-evaluator-input");
                };
                WorkflowIntent::CreateEvaluatorInput {
                    target: NodeId(target),
                    name: name.trim().to_owned(),
                    value_text: value_text.trim().to_owned(),
                }
            }
            AssistantIntentKind::CreateEvaluatorExpression => {
                let Some((name, expression)) = value_text.split_once(':') else {
                    return Err("assistant-error-create-evaluator-expression");
                };
                WorkflowIntent::CreateEvaluatorExpression {
                    target: NodeId(target),
                    name: name.trim().to_owned(),
                    expression: expression.trim().to_owned(),
                }
            }
            AssistantIntentKind::CreateEvaluatorRule => {
                let Some((name, expression)) = value_text.split_once(':') else {
                    return Err("assistant-error-create-evaluator-rule");
                };
                WorkflowIntent::CreateEvaluatorRule {
                    target: NodeId(target),
                    name: name.trim().to_owned(),
                    expression: expression.trim().to_owned(),
                }
            }
            AssistantIntentKind::CreateRuleOverride => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                if fields.len() != 5 {
                    return Err("assistant-error-create-rule-override");
                }
                let Ok(rule) = fields[0].parse::<u64>() else {
                    return Err("assistant-error-create-rule-override");
                };
                WorkflowIntent::CreateRuleOverride {
                    target,
                    rule: NodeId(rule),
                    output_port: fields[1].to_owned(),
                    semantic_key: fields[2].to_owned(),
                    parameter: fields[3].to_owned(),
                    value_text: fields[4].to_owned(),
                }
            }
            AssistantIntentKind::DeleteRuleOverride => {
                WorkflowIntent::DeleteRuleOverride { target }
            }
            AssistantIntentKind::CreateFeatureParameterBinding => {
                self.assistant_create_feature_parameter_binding_intent(target, value_text)?
            }
            AssistantIntentKind::CreatePersistentDimension => {
                self.assistant_create_persistent_dimension_intent(target, value_text)?
            }
            AssistantIntentKind::CreateSpace => {
                let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
                let (Some(purpose), Some(volume_min), Some(volume_max)) = (
                    fields.first().filter(|purpose| !purpose.is_empty()),
                    fields.get(1).and_then(|value| parse_point(value)),
                    fields.get(2).and_then(|value| parse_point(value)),
                ) else {
                    return Err("assistant-error-create-space");
                };
                if fields.len() != 3 {
                    return Err("assistant-error-create-space");
                }
                WorkflowIntent::CreateSpace {
                    target: SpaceId(target),
                    purpose: (*purpose).to_owned(),
                    volume_min,
                    volume_max,
                }
            }
            AssistantIntentKind::CreateClearanceVolume => {
                self.assistant_create_clearance_volume_intent(target, value_text)?
            }
            AssistantIntentKind::CreateJoint => {
                self.assistant_create_joint_intent(target, value_text)?
            }
            AssistantIntentKind::CloneProfileDefinitionAndRepoint => {
                self.assistant_clone_profile_definition_and_repoint_intent(target, value_text)?
            }
            AssistantIntentKind::ConvertEmptyGroupToComponent => {
                let fields = value_text.splitn(3, ':').map(str::trim).collect::<Vec<_>>();
                let [new_definition, new_occurrence, name] = fields.as_slice() else {
                    return Err("assistant-error-convert-empty-group");
                };
                let (Ok(new_definition), Ok(new_occurrence)) =
                    (new_definition.parse::<u64>(), new_occurrence.parse::<u64>())
                else {
                    return Err("assistant-error-convert-empty-group");
                };
                WorkflowIntent::ConvertEmptyGroupToComponent {
                    target: GroupId(target),
                    new_definition: DefinitionId(new_definition),
                    new_occurrence: OccurrenceId(new_occurrence),
                    name: (*name).to_owned(),
                }
            }
            AssistantIntentKind::DeleteFeatureParameterBinding => {
                let Some(parameter_target) =
                    self.assistant_feature_parameter_target(FeatureId(target), value_text.trim())
                else {
                    return Err("assistant-error-delete-feature-parameter-binding");
                };
                WorkflowIntent::DeleteFeatureParameterBinding {
                    target: parameter_target,
                }
            }
            AssistantIntentKind::DeleteJoint => WorkflowIntent::DeleteJoint {
                target: JointId(target),
            },
            AssistantIntentKind::DeleteSpace => WorkflowIntent::DeleteSpace {
                target: SpaceId(target),
            },
            AssistantIntentKind::DeleteClearanceVolume => WorkflowIntent::DeleteClearanceVolume {
                target: ClearanceVolumeId(target),
            },
            AssistantIntentKind::DeletePersistentDimension => {
                WorkflowIntent::DeletePersistentDimension {
                    target: PersistentDimensionId(target),
                }
            }
            AssistantIntentKind::RecomputeFeatureParameter => {
                let Some(parameter_target) =
                    self.assistant_feature_parameter_target(FeatureId(target), value_text.trim())
                else {
                    return Err("assistant-error-recompute-feature-parameter");
                };
                WorkflowIntent::RecomputeFeatureParameter {
                    target: parameter_target,
                }
            }
            AssistantIntentKind::RuleDimension => WorkflowIntent::SetRuleDimension {
                target: NodeId(target),
                value_text: value_text.to_owned(),
            },
            AssistantIntentKind::EvaluatorName => WorkflowIntent::RenameEvaluatorNode {
                target: NodeId(target),
                name: value_text.to_owned(),
            },
            AssistantIntentKind::EvaluatorExpression => WorkflowIntent::SetEvaluatorExpression {
                target: NodeId(target),
                expression: value_text.to_owned(),
            },
            AssistantIntentKind::RuleOutputs => {
                self.assistant_rule_outputs_intent(target, value_text)?
            }
            AssistantIntentKind::FeatureDimension => WorkflowIntent::SetFeatureDimension {
                target: FeatureId(target),
                value_text: value_text.to_owned(),
            },
            AssistantIntentKind::ProfilePoints => {
                let mut points_mm = Vec::new();
                for point in value_text.split(';') {
                    let coordinates = point.split(',').map(str::trim).collect::<Vec<_>>();
                    if coordinates.len() != 2 {
                        return Err("assistant-error-profile-points");
                    }
                    let (Ok(x_mm), Ok(y_mm)) =
                        (coordinates[0].parse::<f64>(), coordinates[1].parse::<f64>())
                    else {
                        return Err("assistant-error-profile-points");
                    };
                    points_mm.push([x_mm, y_mm]);
                }
                WorkflowIntent::SetProfilePoints {
                    target: FeatureId(target),
                    points_mm,
                }
            }
            AssistantIntentKind::DefinitionName => WorkflowIntent::RenameDefinition {
                target: DefinitionId(target),
                name: value_text.to_owned(),
            },
            AssistantIntentKind::OccurrenceVisibility => {
                let Ok(visible) = value_text.trim().parse::<bool>() else {
                    return Err("assistant-error-boolean");
                };
                WorkflowIntent::SetOccurrenceVisibility {
                    target: OccurrenceId(target),
                    visible,
                }
            }
            AssistantIntentKind::TagVisibility => {
                let Ok(visible) = value_text.trim().parse::<bool>() else {
                    return Err("assistant-error-boolean");
                };
                WorkflowIntent::SetTagVisibility {
                    target: TagId(target),
                    visible,
                }
            }
            AssistantIntentKind::OccurrenceTag => {
                let tag = if value_text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    let Ok(tag) = value_text.trim().parse::<u64>() else {
                        return Err("assistant-error-tag");
                    };
                    Some(TagId(tag))
                };
                WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(target),
                    tag,
                }
            }
            AssistantIntentKind::OccurrenceDefinition => {
                let Ok(definition) = value_text.trim().parse::<u64>() else {
                    return Err("assistant-error-definition");
                };
                WorkflowIntent::RepointOccurrence {
                    target: OccurrenceId(target),
                    definition: DefinitionId(definition),
                }
            }
            AssistantIntentKind::OccurrenceParent => {
                let parent = if value_text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    let Ok(parent) = value_text.trim().parse::<u64>() else {
                        return Err("assistant-error-group");
                    };
                    Some(GroupId(parent))
                };
                WorkflowIntent::SetOccurrenceParent {
                    target: OccurrenceId(target),
                    parent,
                }
            }
            AssistantIntentKind::OccurrenceTranslation => {
                let values = value_text.split(',').map(str::trim).collect::<Vec<_>>();
                if values.len() != 3 {
                    return Err("assistant-error-translation");
                }
                WorkflowIntent::SetOccurrenceTranslation {
                    target: OccurrenceId(target),
                    x_mm_text: values[0].to_owned(),
                    y_mm_text: values[1].to_owned(),
                    z_mm_text: values[2].to_owned(),
                }
            }
            AssistantIntentKind::CreateTag => {
                let Some((visible, name)) = value_text.split_once(':') else {
                    return Err("assistant-error-create-tag");
                };
                let Ok(visible) = visible.trim().parse::<bool>() else {
                    return Err("assistant-error-create-tag");
                };
                WorkflowIntent::CreateTag {
                    target: TagId(target),
                    name: name.trim().to_owned(),
                    visible,
                }
            }
            AssistantIntentKind::DeleteTag => WorkflowIntent::DeleteTag {
                target: TagId(target),
            },
            AssistantIntentKind::CreateCollection => WorkflowIntent::CreateCollection {
                target: CollectionId(target),
                name: value_text.to_owned(),
            },
            AssistantIntentKind::DeleteCollection => WorkflowIntent::DeleteCollection {
                target: CollectionId(target),
            },
            AssistantIntentKind::DeleteGroup => WorkflowIntent::DeleteGroup {
                target: GroupId(target),
            },
            AssistantIntentKind::DeleteOccurrence => WorkflowIntent::DeleteOccurrence {
                target: OccurrenceId(target),
            },
            AssistantIntentKind::CreateDefinition => WorkflowIntent::CreateDefinition {
                target: DefinitionId(target),
                name: value_text.to_owned(),
            },
            AssistantIntentKind::DeleteDefinition => WorkflowIntent::DeleteDefinition {
                target: DefinitionId(target),
            },
            AssistantIntentKind::CreateProfileFeature => {
                self.assistant_create_profile_feature_intent(target, value_text)?
            }
            AssistantIntentKind::DeleteProfileFeature => WorkflowIntent::DeleteProfileFeature {
                target: FeatureId(target),
            },
            AssistantIntentKind::CreateGroup => WorkflowIntent::CreateGroup {
                target: GroupId(target),
                name: value_text.to_owned(),
            },
            AssistantIntentKind::CreateOccurrence => {
                let Some((definition, name)) = value_text.split_once(':') else {
                    return Err("assistant-error-create-occurrence");
                };
                let Ok(definition) = definition.trim().parse::<u64>() else {
                    return Err("assistant-error-create-occurrence");
                };
                WorkflowIntent::CreateOccurrence {
                    target: OccurrenceId(target),
                    definition: DefinitionId(definition),
                    name: name.trim().to_owned(),
                }
            }
            AssistantIntentKind::CollectionOccurrences => {
                self.assistant_collection_occurrences_intent(target, value_text)?
            }
            AssistantIntentKind::GroupParent => {
                let parent = if value_text.trim().eq_ignore_ascii_case("none") {
                    None
                } else {
                    let Ok(parent) = value_text.trim().parse::<u64>() else {
                        return Err("assistant-error-group-parent");
                    };
                    Some(GroupId(parent))
                };
                WorkflowIntent::SetGroupParent {
                    target: GroupId(target),
                    parent,
                }
            }
            AssistantIntentKind::GroupTranslation => {
                let values = value_text.split(',').map(str::trim).collect::<Vec<_>>();
                if values.len() != 3 {
                    return Err("assistant-error-group-translation");
                }
                WorkflowIntent::SetGroupTranslation {
                    target: GroupId(target),
                    x_mm_text: values[0].to_owned(),
                    y_mm_text: values[1].to_owned(),
                    z_mm_text: values[2].to_owned(),
                }
            }
        })
    }

    fn assistant_create_feature_parameter_binding_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err("assistant-error-create-feature-parameter-binding");
        }
        let Some(parameter_target) =
            self.assistant_feature_parameter_target(FeatureId(target), fields[0])
        else {
            return Err("assistant-error-create-feature-parameter-binding");
        };
        let Ok(rule) = fields[1].parse::<u64>() else {
            return Err("assistant-error-create-feature-parameter-binding");
        };
        Ok(WorkflowIntent::CreateFeatureParameterBinding {
            target: parameter_target,
            rule: NodeId(rule),
            output_port: fields[2].to_owned(),
            semantic_key: fields[3].to_owned(),
        })
    }

    fn assistant_collection_occurrences_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let occurrence_ids = if value_text.trim().eq_ignore_ascii_case("none") {
            Vec::new()
        } else {
            let parsed = value_text
                .split(',')
                .map(|value| value.trim().parse::<u64>().map(OccurrenceId))
                .collect::<Result<Vec<_>, _>>();
            let Ok(occurrence_ids) = parsed else {
                return Err("assistant-error-collection-occurrences");
            };
            if occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err("assistant-error-collection-occurrences");
            }
            occurrence_ids
        };
        Ok(WorkflowIntent::SetCollectionOccurrences {
            target: CollectionId(target),
            occurrence_ids,
        })
    }

    fn assistant_rule_outputs_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let outputs = if value_text.trim().eq_ignore_ascii_case("none") {
            Vec::new()
        } else {
            let mut outputs = Vec::new();
            for output in value_text.split(';') {
                let Some((port, key)) = output.split_once(':') else {
                    return Err("assistant-error-rule-outputs");
                };
                let Ok(segment) = SlotSegment::new(NodeId(target), port.trim(), key.trim()) else {
                    return Err("assistant-error-rule-outputs");
                };
                let Ok(output) = RuleOutput::new(segment, Vec::new()) else {
                    return Err("assistant-error-rule-outputs");
                };
                outputs.push(output);
            }
            outputs
        };
        Ok(WorkflowIntent::SetRuleOutputs {
            target: NodeId(target),
            outputs,
        })
    }

    fn assistant_create_profile_feature_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let fields = value_text.splitn(3, ':').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("assistant-error-create-profile-feature");
        }
        let Ok(definition) = fields[0].parse::<u64>() else {
            return Err("assistant-error-create-profile-feature");
        };
        let mut points_mm = Vec::new();
        for point in fields[2].split(';') {
            let coordinates = point.split(',').map(str::trim).collect::<Vec<_>>();
            if coordinates.len() != 2 {
                return Err("assistant-error-create-profile-feature");
            }
            let (Ok(x_mm), Ok(y_mm)) =
                (coordinates[0].parse::<f64>(), coordinates[1].parse::<f64>())
            else {
                return Err("assistant-error-create-profile-feature");
            };
            points_mm.push([x_mm, y_mm]);
        }
        Ok(WorkflowIntent::CreateProfileFeature {
            target: FeatureId(target),
            definition: DefinitionId(definition),
            name: fields[1].to_owned(),
            points_mm,
        })
    }

    fn assistant_create_joint_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
        let parse_participant = |value: &str| {
            let fields = value.split(',').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 3 {
                return None;
            }
            let root = NodeId(fields[0].parse::<u64>().ok()?);
            let path =
                SlotPath::new(vec![SlotSegment::new(root, fields[1], fields[2]).ok()?]).ok()?;
            DerivedIdentity::new(root, path).ok()
        };
        let (Some(participant_a), Some(participant_b), Some(volume_min), Some(volume_max)) = (
            fields.first().and_then(|value| parse_participant(value)),
            fields.get(1).and_then(|value| parse_participant(value)),
            fields.get(2).and_then(|value| parse_point(value)),
            fields.get(3).and_then(|value| parse_point(value)),
        ) else {
            return Err("assistant-error-create-joint");
        };
        if fields.len() != 4 {
            return Err("assistant-error-create-joint");
        }
        Ok(WorkflowIntent::CreateJoint {
            target: JointId(target),
            participant_a,
            participant_b,
            volume_min,
            volume_max,
        })
    }

    fn assistant_create_persistent_dimension_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err("assistant-error-create-persistent-dimension");
        }
        let Ok(feature_id) = fields[1].parse::<u64>() else {
            return Err("assistant-error-create-persistent-dimension");
        };
        let Some(dimension_target) =
            self.assistant_feature_parameter_target(FeatureId(feature_id), fields[2])
        else {
            return Err("assistant-error-create-persistent-dimension");
        };
        let unit = match fields[3] {
            "mm" => DimensionDisplayUnit::Millimetres,
            "cm" => DimensionDisplayUnit::Centimetres,
            "in" => DimensionDisplayUnit::Inches,
            _ => {
                return Err("assistant-error-create-persistent-dimension");
            }
        };
        let Ok(decimal_places) = fields[4].parse::<u8>() else {
            return Err("assistant-error-create-persistent-dimension");
        };
        let Ok(presentation) = DimensionPresentation::new(unit, decimal_places) else {
            return Err("assistant-error-create-persistent-dimension");
        };
        Ok(WorkflowIntent::CreatePersistentDimension {
            target: PersistentDimensionId(target),
            name: fields[0].to_owned(),
            dimension_target,
            presentation,
        })
    }

    fn assistant_clone_profile_definition_and_repoint_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let fields = value_text.splitn(5, ':').map(str::trim).collect::<Vec<_>>();
        let [
            source_definition,
            source_feature,
            new_definition,
            new_feature,
            name,
        ] = fields.as_slice()
        else {
            return Err("assistant-error-clone-profile-definition");
        };
        let (Ok(source_definition), Ok(source_feature), Ok(new_definition), Ok(new_feature)) = (
            source_definition.parse::<u64>(),
            source_feature.parse::<u64>(),
            new_definition.parse::<u64>(),
            new_feature.parse::<u64>(),
        ) else {
            return Err("assistant-error-clone-profile-definition");
        };
        Ok(WorkflowIntent::CloneProfileDefinitionAndRepoint {
            target: OccurrenceId(target),
            source_definition: DefinitionId(source_definition),
            source_feature: FeatureId(source_feature),
            new_definition: DefinitionId(new_definition),
            new_feature: FeatureId(new_feature),
            name: (*name).to_owned(),
        })
    }

    fn assistant_create_clearance_volume_intent(
        &self,
        target: u64,
        value_text: &str,
    ) -> Result<WorkflowIntent, &'static str> {
        let fields = value_text.split(':').map(str::trim).collect::<Vec<_>>();
        let (
            Some(owner),
            Some(reason),
            Some(volume_min),
            Some(volume_max),
            Some(tolerance),
            Some(severity),
        ) = (
            fields.first().and_then(|value| value.parse::<u64>().ok()),
            fields.get(1).filter(|reason| !reason.is_empty()),
            fields.get(2).and_then(|value| parse_point(value)),
            fields.get(3).and_then(|value| parse_point(value)),
            fields.get(4).and_then(|value| value.parse::<f64>().ok()),
            fields.get(5).and_then(|value| match *value {
                "advisory" => Some(ClearanceSeverity::Advisory),
                "required" => Some(ClearanceSeverity::Required),
                _ => None,
            }),
        )
        else {
            return Err("assistant-error-create-clearance-volume");
        };
        if fields.len() != 6 {
            return Err("assistant-error-create-clearance-volume");
        }
        Ok(WorkflowIntent::CreateClearanceVolume {
            target: ClearanceVolumeId(target),
            owner: SpaceId(owner),
            reason: (*reason).to_owned(),
            volume_min,
            volume_max,
            tolerance_mm: tolerance,
            severity,
        })
    }

    pub(crate) fn show_outliner_without_assistant(&mut self, ui: &mut egui::Ui) {
        self.show_pocket_properties(ui);
        self.show_classification_dimensions(ui);
        if self.panels.outliner_visible {
            let groups = self.outliner_groups();
            let entries = self.outliner_query();
            section_header(ui, self.palette(), &self.catalog.text("dock-outliner"));
            ui.separator();
            // Bounded so the sections below it stay reachable in the dock.
            egui::ScrollArea::vertical()
                .id_salt("outliner-scroll")
                .max_height(280.0)
                .show(ui, |ui| {
                    if self.show_local_component_outliner(ui) {
                        return;
                    }
                    for group in groups {
                        let label = self.catalog.format(
                            "outliner-group",
                            &BTreeMap::from([
                                ("name", group.name),
                                ("count", group.member_count.to_string()),
                            ]),
                        );
                        let response = ui.selectable_label(
                            self.selection.selected_group == Some(group.id),
                            label,
                        );
                        if response.double_clicked() {
                            self.enter_group_context(group.id);
                        } else if response.clicked() {
                            self.select_group(group.id);
                        }
                    }
                    if !entries.is_empty() {
                        ui.separator();
                    }
                    for mut definition in entries {
                        if definition.occurrences.len() == 1 {
                            let occurrence = definition.occurrences.remove(0);
                            let default_name = format!("{} #1", definition.name);
                            let name = if occurrence.name == default_name {
                                definition.name
                            } else {
                                occurrence.name.clone()
                            };
                            let mut arguments = BTreeMap::from([
                                ("name", name),
                                ("dimensions", definition.specification),
                                (
                                    "visibility",
                                    if occurrence.visible { "◉" } else { "○" }.to_owned(),
                                ),
                            ]);
                            let key = if let Some(group_id) = occurrence.parent {
                                arguments.insert("group", group_id.0.to_string());
                                "outliner-object-grouped"
                            } else {
                                "outliner-object"
                            };
                            let response = ui.selectable_label(
                                self.selection.contains(&occurrence.instance_path),
                                self.catalog.format(key, &arguments),
                            );
                            if response.double_clicked() {
                                self.enter_occurrence_context(occurrence.instance_path.clone());
                            } else if response.clicked() {
                                let additive = ui.input(|input| input.modifiers.shift);
                                self.select_from_outliner(occurrence.instance_path, additive);
                            }
                        } else {
                            let count = definition.occurrences.len();
                            let heading = self.catalog.format(
                                "outliner-component",
                                &BTreeMap::from([
                                    ("name", definition.name),
                                    ("count", count.to_string()),
                                    ("dimensions", definition.specification),
                                ]),
                            );
                            let definition_id = definition.id;
                            let response = egui::CollapsingHeader::new(heading)
                                .id_salt(definition_id.0)
                                .show(ui, |ui| {
                                    for occurrence in definition.occurrences {
                                        let mut arguments = BTreeMap::from([
                                            ("name", occurrence.name),
                                            (
                                                "visibility",
                                                if occurrence.visible { "◉" } else { "○" }
                                                    .to_owned(),
                                            ),
                                        ]);
                                        let key = if let Some(group_id) = occurrence.parent {
                                            arguments.insert("group", group_id.0.to_string());
                                            "outliner-instance-grouped"
                                        } else {
                                            "outliner-instance"
                                        };
                                        let row = ui.selectable_label(
                                            self.selection.contains(&occurrence.instance_path),
                                            self.catalog.format(key, &arguments),
                                        );
                                        if row.double_clicked() {
                                            self.enter_occurrence_context(
                                                occurrence.instance_path.clone(),
                                            );
                                        } else if row.clicked() {
                                            let additive = ui.input(|input| input.modifiers.shift);
                                            self.select_from_outliner(
                                                occurrence.instance_path,
                                                additive,
                                            );
                                        }
                                    }
                                });
                            if response.header_response.clicked() {
                                let additive = ui.input(|input| input.modifiers.shift);
                                self.select_definition(definition_id, additive);
                            }
                        }
                        ui.add_space(2.0);
                    }
                });
        }
    }

    /// Whether the layers lead the dock: once the document has layers,
    /// switching what is visible must not need scrolling past other sections.
    pub(crate) fn layers_lead_dock(&self) -> bool {
        self.document.current().tags().next().is_some()
    }

    /// Layers, saved views and the section plane, separated from the sections
    /// below (`first`) or above it.
    pub(crate) fn show_layers_section(&mut self, ui: &mut egui::Ui, first: bool) {
        if self.panels.tags_visible {
            if !first {
                ui.separator();
            }
            section_header(ui, self.palette(), &self.catalog.text("dock-tags"));
            ui.horizontal_wrapped(|ui| {
                let create_enabled = self.can_begin_tag_creation(None);
                if ui
                    .add_enabled(
                        create_enabled,
                        egui::Button::new(self.catalog.text("tags-create")),
                    )
                    .clicked()
                {
                    self.begin_tag_creation(None);
                }
                let selected_occurrence_ids = self.selected_occurrence_ids();
                let enabled = self.can_begin_tag_creation(Some(&selected_occurrence_ids));
                let create_from_selection = ui.add_enabled(enabled, egui::Button::new("+"));
                name_widget(
                    &create_from_selection,
                    enabled,
                    &self.catalog.text("tags-create-from-selection"),
                );
                if create_from_selection.clicked() {
                    self.begin_tag_creation(Some(selected_occurrence_ids));
                }
                let show_enabled = self.can_set_all_tag_visibility(true);
                let show_all = ui.add_enabled(show_enabled, egui::Button::new("◉"));
                name_widget(&show_all, show_enabled, &self.catalog.text("tags-show-all"));
                if show_all.clicked() {
                    self.set_all_tag_visibility(true);
                }
                let hide_enabled = self.can_set_all_tag_visibility(false);
                let hide_all = ui.add_enabled(hide_enabled, egui::Button::new("○"));
                name_widget(&hide_all, hide_enabled, &self.catalog.text("tags-hide-all"));
                if hide_all.clicked() {
                    self.set_all_tag_visibility(false);
                }
                let invert_enabled = self.can_invert_tag_visibility();
                let invert = ui.add_enabled(invert_enabled, egui::Button::new("◐"));
                name_widget(
                    &invert,
                    invert_enabled,
                    &self.catalog.text("tags-invert-visibility"),
                );
                if invert.clicked() {
                    self.invert_tag_visibility();
                }
                let isolate_selection_enabled = self.can_isolate_selected_tags();
                let isolate_selection =
                    ui.add_enabled(isolate_selection_enabled, egui::Button::new("◇"));
                name_widget(
                    &isolate_selection,
                    isolate_selection_enabled,
                    &self.catalog.text("tags-isolate-selection"),
                );
                if isolate_selection.clicked() {
                    self.isolate_selected_tags();
                }
                let hide_selection_enabled = self.can_hide_selected_tags();
                let hide_selection = ui.add_enabled(hide_selection_enabled, egui::Button::new("◒"));
                name_widget(
                    &hide_selection,
                    hide_selection_enabled,
                    &self.catalog.text("tags-hide-selection"),
                );
                if hide_selection.clicked() {
                    self.hide_selected_tags();
                }
                let show_selection_enabled = self.can_show_selected_tags();
                let show_selection = ui.add_enabled(show_selection_enabled, egui::Button::new("◓"));
                name_widget(
                    &show_selection,
                    show_selection_enabled,
                    &self.catalog.text("tags-show-selection"),
                );
                if show_selection.clicked() {
                    self.show_selected_tags();
                }
                let invert_selection_enabled = self.can_invert_selected_tags();
                let invert_selection =
                    ui.add_enabled(invert_selection_enabled, egui::Button::new("◑"));
                name_widget(
                    &invert_selection,
                    invert_selection_enabled,
                    &self.catalog.text("tags-invert-selection"),
                );
                if invert_selection.clicked() {
                    self.invert_selected_tags();
                }
                let select_matching_enabled = self.can_select_matching_tags();
                let select_matching =
                    ui.add_enabled(select_matching_enabled, egui::Button::new("◆"));
                name_widget(
                    &select_matching,
                    select_matching_enabled,
                    &self.catalog.text("tags-select-matching"),
                );
                if select_matching.clicked() {
                    self.select_matching_tags();
                }
            });
            ui.horizontal_wrapped(|ui| {
                let select_all_tagged_enabled = self.can_select_all_tagged_occurrences();
                let select_all_tagged =
                    ui.add_enabled(select_all_tagged_enabled, egui::Button::new("◈"));
                name_widget(
                    &select_all_tagged,
                    select_all_tagged_enabled,
                    &self.catalog.text("tags-select-all-tagged"),
                );
                if select_all_tagged.clicked() {
                    self.select_all_tagged_occurrences();
                }
                let select_untagged_enabled = self.can_select_untagged_occurrences();
                let select_untagged =
                    ui.add_enabled(select_untagged_enabled, egui::Button::new("○"));
                name_widget(
                    &select_untagged,
                    select_untagged_enabled,
                    &self.catalog.text("tags-select-untagged"),
                );
                if select_untagged.clicked() {
                    self.select_untagged_occurrences();
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.command_enabled(AppCommand::MakeUnique),
                        egui::Button::new(self.catalog.text("model-make-unique")),
                    )
                    .clicked()
                {
                    self.dispatch_command(AppCommand::MakeUnique);
                }
            });
            let scene = self.document.current().scene_query();
            let hidden = scene.iter().filter(|item| !item.visible).count();
            ui.label(self.catalog.format(
                "tags-visibility",
                &BTreeMap::from([
                    ("hidden", hidden.to_string()),
                    ("total", scene.len().to_string()),
                ]),
            ));
            let active_scene = self.active_scene_query();
            for (id, name, mut visible, count) in self.tag_rows() {
                let label = self.catalog.format(
                    "tags-row",
                    &BTreeMap::from([("name", name.clone()), ("count", count.to_string())]),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut visible, label).changed() {
                        self.set_tag_visibility(id, visible);
                    }
                    let isolate_label = self
                        .catalog
                        .format("tags-isolate", &BTreeMap::from([("name", name.clone())]));
                    let isolate_enabled = self.can_isolate_tag(id);
                    let isolate = ui.add_enabled(isolate_enabled, egui::Button::new("◎"));
                    name_widget(&isolate, isolate_enabled, &isolate_label);
                    if isolate.clicked() {
                        self.isolate_tag(id);
                    }
                    let select_label = self
                        .catalog
                        .format("tags-select", &BTreeMap::from([("name", name.clone())]));
                    let select_enabled = self.can_select_tag_occurrences(id, &active_scene);
                    let select = ui.add_enabled(select_enabled, egui::Button::new("▣"));
                    name_widget(&select, select_enabled, &select_label);
                    if select.clicked() {
                        self.select_tag_occurrences(id);
                    }
                    let clear_label = self
                        .catalog
                        .format("tags-clear", &BTreeMap::from([("name", name.clone())]));
                    let clear_enabled = self.can_clear_tag(id);
                    let clear = ui.add_enabled(clear_enabled, egui::Button::new("×"));
                    name_widget(&clear, clear_enabled, &clear_label);
                    if clear.clicked() {
                        self.begin_tag_clear(id);
                    }
                    let rename_label = self
                        .catalog
                        .format("tags-rename", &BTreeMap::from([("name", name.clone())]));
                    let rename_enabled = self.tag_rename_source_plan(id).is_some();
                    let rename = ui.add_enabled(rename_enabled, egui::Button::new("✎"));
                    name_widget(&rename, rename_enabled, &rename_label);
                    if rename.clicked() {
                        self.begin_tag_rename(id);
                    }
                    let delete_label = self
                        .catalog
                        .format("tags-delete", &BTreeMap::from([("name", name.clone())]));
                    let delete_enabled = self.can_delete_tag(id);
                    let delete = ui.add_enabled(delete_enabled, egui::Button::new("⌫"));
                    name_widget(&delete, delete_enabled, &delete_label);
                    if delete.clicked() {
                        self.begin_tag_deletion(id);
                    }
                    let remove_label = self.catalog.format(
                        "tags-remove-selection",
                        &BTreeMap::from([("name", name.clone())]),
                    );
                    let remove_enabled = self.can_remove_selection_from_tag(id);
                    let remove = ui.add_enabled(remove_enabled, egui::Button::new("−"));
                    name_widget(&remove, remove_enabled, &remove_label);
                    if remove.clicked() {
                        self.remove_selection_from_tag(id);
                    }
                    let assign_label = self
                        .catalog
                        .format("tags-assign-selection", &BTreeMap::from([("name", name)]));
                    let assign_enabled = self.can_assign_selection_to_tag(id);
                    let assign = ui.add_enabled(assign_enabled, egui::Button::new("+"));
                    name_widget(&assign, assign_enabled, &assign_label);
                    if assign.clicked() {
                        self.assign_selection_to_tag(id);
                    }
                });
            }
            ui.horizontal(|ui| {
                self.command_button(ui, AppCommand::Hide);
                self.command_button(ui, AppCommand::Unhide);
            });
            self.saved_views_ui(ui);
            self.section_ui(ui);
            if first {
                ui.separator();
            }
        }
    }
}

pub(crate) fn assistant_body_bounds_from_snapshot(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
) -> BTreeMap<InstancePath, (DefinitionId, [Vec3; 2])> {
    CanonicalInteractionProjection::from_snapshot(snapshot)
        .occurrences()
        .iter()
        .filter(|occurrence| occurrence.visible)
        .filter_map(|occurrence| {
            let [minimum, maximum] = assistant_definition_local_bounds(
                snapshot,
                exact_results,
                occurrence.body.definition_id,
                occurrence.local_box,
            )?;
            let size = maximum - minimum;
            let bounds = bounds_of(box_corners(size.x, size.y, size.z).into_iter().map(
                |corner| {
                    transform_model_point(occurrence.canonical_world_transform, corner + minimum)
                },
            ))?;
            Some((
                occurrence.instance_path.clone(),
                (occurrence.body.definition_id, bounds),
            ))
        })
        .collect()
}

pub(crate) fn assistant_occurrence_records_from_snapshot(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
) -> Vec<(InstancePath, serde_json::Value)> {
    let body_bounds = assistant_body_bounds_from_snapshot(snapshot, exact_results);
    snapshot
        .scene_query()
        .into_iter()
        .map(|occurrence| {
            let instance_path = occurrence.instance_path.clone();
            let bounds = body_bounds
                .get(&instance_path)
                .map(|(_, bounds)| *bounds)
                .or_else(|| assistant_mesh_body_bounds(snapshot, &occurrence));
            let record = serde_json::json!({
                "occurrence_id": occurrence.occurrence_id.0,
                "instance_path": KetchupApp::assistant_instance_path_value(snapshot, &instance_path),
                "definition_id": occurrence.definition_id.0,
                "name": occurrence.occurrence_name,
                "visible": occurrence.visible,
                "copyable": instance_path.is_root(),
                "bounds_mm": bounds.map(|[minimum, maximum]| serde_json::json!({
                    "min": [minimum.x, minimum.y, minimum.z],
                    "max": [maximum.x, maximum.y, maximum.z],
                })),
            });
            (instance_path, record)
        })
        .collect()
}

/// A point written as `x,y,z` in millimetres.
fn parse_point(value: &str) -> Option<[f64; 3]> {
    let values = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    <[f64; 3]>::try_from(values).ok()
}
