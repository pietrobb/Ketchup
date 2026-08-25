use crate::document::{
    AuthoritativeDependency, BottleControlDimension, BottleEdgeFinishKind, CanonicalCommand,
    CanonicalOverride, CloneDefinitionPlan, CollectionId, CommandBatch, ConvertGroupPlan,
    DefinitionId, DerivedIdentity, Dimension, DocumentStore, EvaluationIdentity, FeatureId,
    FeatureKind, FeatureParameterBinding, FeatureParameterTarget, GroupId, NodeId, OccurrenceId,
    PersistentDimension, PersistentDimensionTarget, Proposal, ProposalAssumption, ProposalBudget,
    ProposalConfirmation, ProposalContext, ProposalGoal, ProposalPrepareError, ProposalPrincipal,
    ProposalRisk, SlotPath, SlotResolution, SlotSegment, TagId, Transform,
};
use crate::graph::ExpressionAst;
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntentCapability {
    CreateEvaluatorInput,
    CreateEvaluatorExpression,
    CreateEvaluatorRule,
    CreateRuleOverride,
    DeleteRuleOverride,
    CreateFeatureParameterBinding,
    DeleteFeatureParameterBinding,
    CreatePersistentDimension,
    CreateSpace,
    CreateClearanceVolume,
    CreateJoint,
    RecomputeFeatureParameter,
    DeleteJoint,
    DeleteSpace,
    DeleteClearanceVolume,
    DeletePersistentDimension,
    SetRuleDimension,
    RenameEvaluatorNode,
    SetEvaluatorExpression,
    SetRuleOutputs,
    SetFeatureDimension,
    SetBottleControlDimension,
    SetBottleEdgeFinishKind,
    SetProfilePoints,
    RenameDefinition,
    SetOccurrenceVisibility,
    SetOccurrenceTranslation,
    AtomicMultiCommandEdit,
    SetOccurrenceTag,
    SetTagVisibility,
    RepointOccurrence,
    SetOccurrenceParent,
    SetGroupTranslation,
    SetGroupParent,
    SetCollectionOccurrences,
    CreateTag,
    DeleteTag,
    CreateCollection,
    DeleteCollection,
    DeleteGroup,
    DeleteOccurrence,
    CreateDefinition,
    DeleteDefinition,
    CreateProfileFeature,
    DeleteProfileFeature,
    CreateGroup,
    CreateOccurrence,
    CloneProfileDefinitionAndRepoint,
    ConvertEmptyGroupToComponent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestingPrincipal {
    LocalAssistant,
    Plugin(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentGrant {
    principal: RequestingPrincipal,
    capabilities: BTreeSet<IntentCapability>,
}

impl IntentGrant {
    #[must_use]
    pub fn new(
        principal: RequestingPrincipal,
        capabilities: impl IntoIterator<Item = IntentCapability>,
    ) -> Self {
        Self {
            principal,
            capabilities: capabilities.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn m7a_local_assistant() -> Self {
        Self::new(
            RequestingPrincipal::LocalAssistant,
            [
                IntentCapability::CreateEvaluatorInput,
                IntentCapability::CreateEvaluatorExpression,
                IntentCapability::CreateEvaluatorRule,
                IntentCapability::CreateRuleOverride,
                IntentCapability::DeleteRuleOverride,
                IntentCapability::CreateFeatureParameterBinding,
                IntentCapability::DeleteFeatureParameterBinding,
                IntentCapability::CreatePersistentDimension,
                IntentCapability::CreateSpace,
                IntentCapability::CreateClearanceVolume,
                IntentCapability::CreateJoint,
                IntentCapability::RecomputeFeatureParameter,
                IntentCapability::DeleteJoint,
                IntentCapability::DeleteSpace,
                IntentCapability::DeleteClearanceVolume,
                IntentCapability::DeletePersistentDimension,
                IntentCapability::SetRuleDimension,
                IntentCapability::RenameEvaluatorNode,
                IntentCapability::SetEvaluatorExpression,
                IntentCapability::SetRuleOutputs,
                IntentCapability::SetFeatureDimension,
                IntentCapability::SetBottleControlDimension,
                IntentCapability::SetBottleEdgeFinishKind,
                IntentCapability::SetProfilePoints,
                IntentCapability::RenameDefinition,
                IntentCapability::SetOccurrenceVisibility,
                IntentCapability::SetOccurrenceTranslation,
                IntentCapability::AtomicMultiCommandEdit,
                IntentCapability::SetOccurrenceTag,
                IntentCapability::SetTagVisibility,
                IntentCapability::RepointOccurrence,
                IntentCapability::SetOccurrenceParent,
                IntentCapability::SetGroupTranslation,
                IntentCapability::SetGroupParent,
                IntentCapability::SetCollectionOccurrences,
                IntentCapability::CreateTag,
                IntentCapability::DeleteTag,
                IntentCapability::CreateCollection,
                IntentCapability::DeleteCollection,
                IntentCapability::DeleteGroup,
                IntentCapability::DeleteOccurrence,
                IntentCapability::CreateDefinition,
                IntentCapability::DeleteDefinition,
                IntentCapability::CreateProfileFeature,
                IntentCapability::DeleteProfileFeature,
                IntentCapability::CreateGroup,
                IntentCapability::CreateOccurrence,
                IntentCapability::CloneProfileDefinitionAndRepoint,
                IntentCapability::ConvertEmptyGroupToComponent,
            ],
        )
    }

    #[must_use]
    pub fn m7b_plugin(
        principal_id: u64,
        capabilities: impl IntoIterator<Item = IntentCapability>,
    ) -> Self {
        Self::new(RequestingPrincipal::Plugin(principal_id), capabilities)
    }

    #[must_use]
    pub const fn principal(&self) -> RequestingPrincipal {
        self.principal
    }

    #[must_use]
    pub fn capabilities(&self) -> &BTreeSet<IntentCapability> {
        &self.capabilities
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowIntent {
    CreateEvaluatorInput {
        target: NodeId,
        name: String,
        value_text: String,
    },
    CreateEvaluatorExpression {
        target: NodeId,
        name: String,
        expression: String,
    },
    CreateEvaluatorRule {
        target: NodeId,
        name: String,
        expression: String,
    },
    CreateRuleOverride {
        target: u64,
        rule: NodeId,
        output_port: String,
        semantic_key: String,
        parameter: String,
        value_text: String,
    },
    DeleteRuleOverride {
        target: u64,
    },
    CreateFeatureParameterBinding {
        target: FeatureParameterTarget,
        rule: NodeId,
        output_port: String,
        semantic_key: String,
    },
    DeleteFeatureParameterBinding {
        target: FeatureParameterTarget,
    },
    CreatePersistentDimension {
        target: crate::document::PersistentDimensionId,
        name: String,
        dimension_target: FeatureParameterTarget,
        presentation: crate::document::DimensionPresentation,
    },
    CreateSpace {
        target: crate::space::SpaceId,
        purpose: String,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
    },
    CreateClearanceVolume {
        target: crate::space::ClearanceVolumeId,
        owner: crate::space::SpaceId,
        reason: String,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
        tolerance_mm: f64,
        severity: crate::space::ClearanceSeverity,
    },
    CreateJoint {
        target: crate::prismatic::JointId,
        participant_a: DerivedIdentity,
        participant_b: DerivedIdentity,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
    },
    RecomputeFeatureParameter {
        target: FeatureParameterTarget,
    },
    DeleteJoint {
        target: crate::prismatic::JointId,
    },
    DeleteSpace {
        target: crate::space::SpaceId,
    },
    DeleteClearanceVolume {
        target: crate::space::ClearanceVolumeId,
    },
    DeletePersistentDimension {
        target: crate::document::PersistentDimensionId,
    },
    SetRuleDimension {
        target: NodeId,
        value_text: String,
    },
    RenameEvaluatorNode {
        target: NodeId,
        name: String,
    },
    SetEvaluatorExpression {
        target: NodeId,
        expression: String,
    },
    SetRuleOutputs {
        target: NodeId,
        outputs: Vec<crate::document::RuleOutput>,
    },
    SetFeatureDimension {
        target: FeatureId,
        value_text: String,
    },
    SetBottleControlDimension {
        target: FeatureId,
        control: BottleControlDimension,
        value_text: String,
    },
    SetBottleEdgeFinishKind {
        target: FeatureId,
        kind: BottleEdgeFinishKind,
    },
    SetProfilePoints {
        target: FeatureId,
        points_mm: Vec<[f64; 2]>,
    },
    RenameDefinition {
        target: DefinitionId,
        name: String,
    },
    SetOccurrenceVisibility {
        target: OccurrenceId,
        visible: bool,
    },
    SetOccurrenceTranslation {
        target: OccurrenceId,
        x_mm_text: String,
        y_mm_text: String,
        z_mm_text: String,
    },
    AtomicMultiCommandEdit {
        target: OccurrenceId,
        x_mm_text: String,
        y_mm_text: String,
        z_mm_text: String,
        tag: TagId,
        name: String,
    },
    SetOccurrenceTag {
        target: OccurrenceId,
        tag: Option<TagId>,
    },
    SetTagVisibility {
        target: TagId,
        visible: bool,
    },
    RepointOccurrence {
        target: OccurrenceId,
        definition: DefinitionId,
    },
    SetOccurrenceParent {
        target: OccurrenceId,
        parent: Option<GroupId>,
    },
    SetGroupTranslation {
        target: GroupId,
        x_mm_text: String,
        y_mm_text: String,
        z_mm_text: String,
    },
    SetGroupParent {
        target: GroupId,
        parent: Option<GroupId>,
    },
    SetCollectionOccurrences {
        target: CollectionId,
        occurrence_ids: Vec<OccurrenceId>,
    },
    CreateTag {
        target: TagId,
        name: String,
        visible: bool,
    },
    DeleteTag {
        target: TagId,
    },
    CreateCollection {
        target: CollectionId,
        name: String,
    },
    DeleteCollection {
        target: CollectionId,
    },
    DeleteGroup {
        target: GroupId,
    },
    DeleteOccurrence {
        target: OccurrenceId,
    },
    CreateDefinition {
        target: DefinitionId,
        name: String,
    },
    DeleteDefinition {
        target: DefinitionId,
    },
    CreateProfileFeature {
        target: FeatureId,
        definition: DefinitionId,
        name: String,
        points_mm: Vec<[f64; 2]>,
    },
    DeleteProfileFeature {
        target: FeatureId,
    },
    CreateGroup {
        target: GroupId,
        name: String,
    },
    CreateOccurrence {
        target: OccurrenceId,
        definition: DefinitionId,
        name: String,
    },
    CloneProfileDefinitionAndRepoint {
        target: OccurrenceId,
        source_definition: DefinitionId,
        source_feature: FeatureId,
        new_definition: DefinitionId,
        new_feature: FeatureId,
        name: String,
    },
    ConvertEmptyGroupToComponent {
        target: GroupId,
        new_definition: DefinitionId,
        new_occurrence: OccurrenceId,
        name: String,
    },
}

impl WorkflowIntent {
    const fn required_capability(&self) -> IntentCapability {
        match self {
            Self::CreateEvaluatorInput { .. } => IntentCapability::CreateEvaluatorInput,
            Self::CreateEvaluatorExpression { .. } => IntentCapability::CreateEvaluatorExpression,
            Self::CreateEvaluatorRule { .. } => IntentCapability::CreateEvaluatorRule,
            Self::CreateRuleOverride { .. } => IntentCapability::CreateRuleOverride,
            Self::DeleteRuleOverride { .. } => IntentCapability::DeleteRuleOverride,
            Self::CreateFeatureParameterBinding { .. } => {
                IntentCapability::CreateFeatureParameterBinding
            }
            Self::DeleteFeatureParameterBinding { .. } => {
                IntentCapability::DeleteFeatureParameterBinding
            }
            Self::CreatePersistentDimension { .. } => IntentCapability::CreatePersistentDimension,
            Self::CreateSpace { .. } => IntentCapability::CreateSpace,
            Self::CreateClearanceVolume { .. } => IntentCapability::CreateClearanceVolume,
            Self::CreateJoint { .. } => IntentCapability::CreateJoint,
            Self::RecomputeFeatureParameter { .. } => IntentCapability::RecomputeFeatureParameter,
            Self::DeleteJoint { .. } => IntentCapability::DeleteJoint,
            Self::DeleteSpace { .. } => IntentCapability::DeleteSpace,
            Self::DeleteClearanceVolume { .. } => IntentCapability::DeleteClearanceVolume,
            Self::DeletePersistentDimension { .. } => IntentCapability::DeletePersistentDimension,
            Self::SetRuleDimension { .. } => IntentCapability::SetRuleDimension,
            Self::RenameEvaluatorNode { .. } => IntentCapability::RenameEvaluatorNode,
            Self::SetEvaluatorExpression { .. } => IntentCapability::SetEvaluatorExpression,
            Self::SetRuleOutputs { .. } => IntentCapability::SetRuleOutputs,
            Self::SetFeatureDimension { .. } => IntentCapability::SetFeatureDimension,
            Self::SetBottleControlDimension { .. } => IntentCapability::SetBottleControlDimension,
            Self::SetBottleEdgeFinishKind { .. } => IntentCapability::SetBottleEdgeFinishKind,
            Self::SetProfilePoints { .. } => IntentCapability::SetProfilePoints,
            Self::RenameDefinition { .. } => IntentCapability::RenameDefinition,
            Self::SetOccurrenceVisibility { .. } => IntentCapability::SetOccurrenceVisibility,
            Self::SetOccurrenceTranslation { .. } => IntentCapability::SetOccurrenceTranslation,
            Self::AtomicMultiCommandEdit { .. } => IntentCapability::AtomicMultiCommandEdit,
            Self::SetOccurrenceTag { .. } => IntentCapability::SetOccurrenceTag,
            Self::SetTagVisibility { .. } => IntentCapability::SetTagVisibility,
            Self::RepointOccurrence { .. } => IntentCapability::RepointOccurrence,
            Self::SetOccurrenceParent { .. } => IntentCapability::SetOccurrenceParent,
            Self::SetGroupTranslation { .. } => IntentCapability::SetGroupTranslation,
            Self::SetGroupParent { .. } => IntentCapability::SetGroupParent,
            Self::SetCollectionOccurrences { .. } => IntentCapability::SetCollectionOccurrences,
            Self::CreateTag { .. } => IntentCapability::CreateTag,
            Self::DeleteTag { .. } => IntentCapability::DeleteTag,
            Self::CreateCollection { .. } => IntentCapability::CreateCollection,
            Self::DeleteCollection { .. } => IntentCapability::DeleteCollection,
            Self::DeleteGroup { .. } => IntentCapability::DeleteGroup,
            Self::DeleteOccurrence { .. } => IntentCapability::DeleteOccurrence,
            Self::CreateDefinition { .. } => IntentCapability::CreateDefinition,
            Self::DeleteDefinition { .. } => IntentCapability::DeleteDefinition,
            Self::CreateProfileFeature { .. } => IntentCapability::CreateProfileFeature,
            Self::DeleteProfileFeature { .. } => IntentCapability::DeleteProfileFeature,
            Self::CreateGroup { .. } => IntentCapability::CreateGroup,
            Self::CreateOccurrence { .. } => IntentCapability::CreateOccurrence,
            Self::CloneProfileDefinitionAndRepoint { .. } => {
                IntentCapability::CloneProfileDefinitionAndRepoint
            }
            Self::ConvertEmptyGroupToComponent { .. } => {
                IntentCapability::ConvertEmptyGroupToComponent
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntentRequest {
    pub grant: IntentGrant,
    pub intent: WorkflowIntent,
    pub requested_budget: ProposalBudget,
}

impl IntentRequest {
    #[must_use]
    pub fn m7a(intent: WorkflowIntent) -> Self {
        let requested_budget = match &intent {
            WorkflowIntent::CreateProfileFeature { .. }
            | WorkflowIntent::DeleteProfileFeature { .. } => ProposalBudget::M18C_CREATE_FEATURE,
            WorkflowIntent::CloneProfileDefinitionAndRepoint { .. }
            | WorkflowIntent::ConvertEmptyGroupToComponent { .. } => {
                ProposalBudget::M18C_CLONE_PROFILE_DEFINITION
            }
            WorkflowIntent::AtomicMultiCommandEdit { .. } => {
                ProposalBudget::T18_ATOMIC_MULTI_COMMAND_EDIT
            }
            _ => ProposalBudget::M7A_SINGLE_CHANGE,
        };
        Self {
            grant: IntentGrant::m7a_local_assistant(),
            intent,
            requested_budget,
        }
    }
}

pub fn propose_intent(
    store: &DocumentStore,
    request: IntentRequest,
) -> Result<Proposal, IntentError> {
    let required = request.intent.required_capability();
    if !request.grant.capabilities.contains(&required) {
        return Err(IntentError::CapabilityDenied(required));
    }
    if let WorkflowIntent::AtomicMultiCommandEdit {
        target,
        x_mm_text,
        y_mm_text,
        z_mm_text,
        tag,
        name,
    } = &request.intent
    {
        let parse = |value: &str| {
            value
                .trim()
                .parse::<f64>()
                .map_err(|_| crate::document::CanonicalError::InvalidTransform)
        };
        let snapshot = store.current();
        let mut matrix = *snapshot
            .occurrence(*target)
            .ok_or(crate::document::CanonicalError::OccurrenceNotFound(*target))?
            .transform()
            .matrix();
        matrix[3] = parse(x_mm_text)?;
        matrix[7] = parse(y_mm_text)?;
        matrix[11] = parse(z_mm_text)?;
        let batch = CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: *target,
                transform: Transform::from_matrix(matrix)?,
            },
            CanonicalCommand::SetOccurrenceTag {
                id: *target,
                tag: Some(*tag),
            },
            CanonicalCommand::RenameEntity {
                id: *target,
                name: name.clone(),
            },
        ]);
        return store
            .prepare_proposal_with_context(
                batch,
                ProposalContext {
                    principal: match request.grant.principal {
                        RequestingPrincipal::LocalAssistant => ProposalPrincipal::LocalAssistant,
                        RequestingPrincipal::Plugin(id) => ProposalPrincipal::Plugin(id),
                    },
                    goal: ProposalGoal::AtomicMultiCommandEdit(*target),
                    assumptions: vec![
                        ProposalAssumption::TargetExists(AuthoritativeDependency::Occurrence(
                            *target,
                        )),
                        ProposalAssumption::TargetExists(AuthoritativeDependency::Tag(*tag)),
                    ],
                    risk: ProposalRisk::Standard,
                    confirmation: ProposalConfirmation::ReviewRequired,
                    requested_budget: request.requested_budget,
                },
            )
            .map_err(IntentError::Proposal);
    }
    let requested_secondary_targets = match &request.intent {
        WorkflowIntent::SetOccurrenceTag { tag: Some(tag), .. } => {
            vec![AuthoritativeDependency::Tag(*tag)]
        }
        WorkflowIntent::RepointOccurrence { definition, .. } => {
            vec![AuthoritativeDependency::Definition(*definition)]
        }
        WorkflowIntent::SetOccurrenceParent {
            parent: Some(parent),
            ..
        }
        | WorkflowIntent::SetGroupParent {
            parent: Some(parent),
            ..
        } => vec![AuthoritativeDependency::Group(*parent)],
        WorkflowIntent::SetCollectionOccurrences { occurrence_ids, .. } => occurrence_ids
            .iter()
            .copied()
            .map(AuthoritativeDependency::Occurrence)
            .collect(),
        WorkflowIntent::CreateOccurrence { definition, .. }
        | WorkflowIntent::CreateProfileFeature { definition, .. } => {
            vec![AuthoritativeDependency::Definition(*definition)]
        }
        WorkflowIntent::CreateRuleOverride { rule, .. } => {
            vec![AuthoritativeDependency::EvaluatorNode(*rule)]
        }
        WorkflowIntent::CreateFeatureParameterBinding { target, rule, .. } => vec![
            AuthoritativeDependency::Feature(target.feature_id),
            AuthoritativeDependency::EvaluatorNode(*rule),
        ],
        WorkflowIntent::CreatePersistentDimension {
            dimension_target, ..
        } => vec![AuthoritativeDependency::Feature(
            dimension_target.feature_id,
        )],
        WorkflowIntent::CreateClearanceVolume { owner, .. } => {
            vec![AuthoritativeDependency::Space(*owner)]
        }
        WorkflowIntent::CloneProfileDefinitionAndRepoint {
            source_definition,
            source_feature,
            ..
        } => vec![
            AuthoritativeDependency::Definition(*source_definition),
            AuthoritativeDependency::Feature(*source_feature),
        ],
        WorkflowIntent::CreateJoint {
            participant_a,
            participant_b,
            ..
        } => BTreeSet::from([
            AuthoritativeDependency::EvaluatorNode(participant_a.root_rule_node_id),
            AuthoritativeDependency::EvaluatorNode(participant_b.root_rule_node_id),
        ])
        .into_iter()
        .collect(),
        WorkflowIntent::CreateEvaluatorExpression { expression, .. }
        | WorkflowIntent::CreateEvaluatorRule { expression, .. }
        | WorkflowIntent::SetEvaluatorExpression { expression, .. } => {
            ExpressionAst::parse(expression).map_or_else(
                |_| Vec::new(),
                |expression| {
                    expression
                        .dependencies()
                        .into_iter()
                        .map(AuthoritativeDependency::EvaluatorNode)
                        .collect()
                },
            )
        }
        _ => Vec::new(),
    };
    let requested_missing_targets = match &request.intent {
        WorkflowIntent::CloneProfileDefinitionAndRepoint {
            new_definition,
            new_feature,
            ..
        } => vec![
            AuthoritativeDependency::Definition(*new_definition),
            AuthoritativeDependency::Feature(*new_feature),
        ],
        WorkflowIntent::ConvertEmptyGroupToComponent {
            new_definition,
            new_occurrence,
            ..
        } => vec![
            AuthoritativeDependency::Definition(*new_definition),
            AuthoritativeDependency::Occurrence(*new_occurrence),
        ],
        _ => Vec::new(),
    };

    let (goal, target, command) = match request.intent {
        WorkflowIntent::CreateEvaluatorInput {
            target,
            name,
            value_text,
        } => (
            ProposalGoal::CreateEvaluatorInput(target),
            AuthoritativeDependency::EvaluatorNode(target),
            CanonicalCommand::CreateEvaluatorNode {
                id: target,
                name,
                dimension: Dimension::from_decimal(value_text)?,
                dependencies: Vec::new(),
            },
        ),
        WorkflowIntent::CreateEvaluatorExpression {
            target,
            name,
            expression,
        } => (
            ProposalGoal::CreateEvaluatorExpression(target),
            AuthoritativeDependency::EvaluatorNode(target),
            CanonicalCommand::CreateExpressionNode {
                id: target,
                name,
                expression,
            },
        ),
        WorkflowIntent::CreateEvaluatorRule {
            target,
            name,
            expression,
        } => (
            ProposalGoal::CreateEvaluatorRule(target),
            AuthoritativeDependency::EvaluatorNode(target),
            CanonicalCommand::CreateRuleNode {
                id: target,
                name,
                expression,
                input_ports: Vec::new(),
                output_ports: vec![
                    crate::document::PortSpec::number("result")
                        .map_err(crate::document::CanonicalError::Graph)?,
                ],
                outputs: Vec::new(),
                override_parameters: Vec::new(),
            },
        ),
        WorkflowIntent::CreateRuleOverride {
            target,
            rule,
            output_port,
            semantic_key,
            parameter,
            value_text,
        } => {
            if store.current().override_by_id(target).is_some() {
                return Err(crate::document::CanonicalError::OverrideAlreadyExists(target).into());
            }
            let identity = DerivedIdentity::new(
                rule,
                SlotPath::new(vec![
                    SlotSegment::new(rule, output_port, semantic_key)
                        .map_err(crate::document::CanonicalError::Graph)?,
                ])
                .map_err(crate::document::CanonicalError::Graph)?,
            )
            .map_err(crate::document::CanonicalError::Graph)?;
            let snapshot = store.current();
            if snapshot.resolve_slot(&identity) != SlotResolution::Resolved {
                return Err(crate::document::CanonicalError::UnresolvedDerivedOutput.into());
            }
            if !snapshot.evaluator_node(rule).is_some_and(|node| {
                node.allowed_parameters()
                    .iter()
                    .any(|spec| spec.name() == parameter)
            }) {
                return Err(crate::document::CanonicalError::UndeclaredOverrideParameter.into());
            }
            let value = value_text.parse::<f64>().map_err(|_| {
                crate::document::CanonicalError::Graph(
                    crate::document::GraphError::NonFiniteOverride,
                )
            })?;
            let value = CanonicalOverride::new(
                target,
                identity,
                parameter,
                value,
                SlotResolution::Resolved,
            )
            .map_err(crate::document::CanonicalError::Graph)?;
            (
                ProposalGoal::CreateRuleOverride(target),
                AuthoritativeDependency::Override(target),
                CanonicalCommand::UpsertOverride(value),
            )
        }
        WorkflowIntent::DeleteRuleOverride { target } => (
            ProposalGoal::DeleteRuleOverride(target),
            AuthoritativeDependency::Override(target),
            CanonicalCommand::DeleteOverride { id: target },
        ),
        WorkflowIntent::CreateFeatureParameterBinding {
            target,
            rule,
            output_port,
            semantic_key,
        } => {
            if store.current().feature_parameter_binding(target).is_some() {
                return Err(
                    crate::document::CanonicalError::InvalidFeatureParameterBinding(target).into(),
                );
            }
            let derived_from = DerivedIdentity::new(
                rule,
                SlotPath::new(vec![
                    SlotSegment::new(rule, output_port, semantic_key)
                        .map_err(crate::document::CanonicalError::Graph)?,
                ])
                .map_err(crate::document::CanonicalError::Graph)?,
            )
            .map_err(crate::document::CanonicalError::Graph)?;
            (
                ProposalGoal::CreateFeatureParameterBinding(target),
                AuthoritativeDependency::FeatureParameterBinding(target),
                CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                    target,
                    derived_from,
                }),
            )
        }
        WorkflowIntent::DeleteFeatureParameterBinding { target } => (
            ProposalGoal::DeleteFeatureParameterBinding(target),
            AuthoritativeDependency::FeatureParameterBinding(target),
            CanonicalCommand::DeleteFeatureParameterBinding { target },
        ),
        WorkflowIntent::CreatePersistentDimension {
            target,
            name,
            dimension_target,
            presentation,
        } => {
            let snapshot = store.current();
            if snapshot.persistent_dimension(target).is_some() {
                return Err(
                    crate::document::CanonicalError::PersistentDimensionAlreadyExists(target)
                        .into(),
                );
            }
            if !snapshot.has_feature_parameter(dimension_target) {
                return Err(
                    crate::document::CanonicalError::InvalidPersistentDimensionTarget.into(),
                );
            }
            (
                ProposalGoal::CreatePersistentDimension(target),
                AuthoritativeDependency::PersistentDimension(target),
                CanonicalCommand::UpsertPersistentDimension(PersistentDimension::new(
                    target,
                    name,
                    PersistentDimensionTarget::FeatureParameter(dimension_target),
                    presentation,
                )?),
            )
        }
        WorkflowIntent::CreateSpace {
            target,
            purpose,
            volume_min,
            volume_max,
        } => {
            if store.current().space(target).is_some() {
                return Err(crate::document::CanonicalError::SpaceAlreadyExists(target).into());
            }
            (
                ProposalGoal::CreateSpace(target),
                AuthoritativeDependency::Space(target),
                CanonicalCommand::UpsertSpace(
                    crate::space::CanonicalSpace::new(
                        target,
                        purpose,
                        crate::prismatic::Aabb::new(volume_min, volume_max)
                            .map_err(crate::document::CanonicalError::from)?,
                        Vec::new(),
                        Vec::new(),
                    )
                    .map_err(crate::document::CanonicalError::from)?,
                ),
            )
        }
        WorkflowIntent::CreateClearanceVolume {
            target,
            owner,
            reason,
            volume_min,
            volume_max,
            tolerance_mm,
            severity,
        } => {
            let snapshot = store.current();
            if snapshot.clearance_volume(target).is_some() {
                return Err(
                    crate::document::CanonicalError::ClearanceVolumeAlreadyExists(target).into(),
                );
            }
            if snapshot.space(owner).is_none() {
                return Err(crate::document::CanonicalError::SpaceNotFound(owner).into());
            }
            (
                ProposalGoal::CreateClearanceVolume(target),
                AuthoritativeDependency::ClearanceVolume(target),
                CanonicalCommand::UpsertClearanceVolume(
                    crate::space::CanonicalClearanceVolume::new(
                        target,
                        crate::space::ClearanceOwner::Space(owner),
                        reason,
                        crate::prismatic::Aabb::new(volume_min, volume_max)
                            .map_err(crate::document::CanonicalError::from)?,
                        crate::prismatic::TolerancePolicy::new(tolerance_mm)
                            .map_err(crate::document::CanonicalError::from)?,
                        severity,
                        None,
                    )
                    .map_err(crate::document::CanonicalError::from)?,
                ),
            )
        }
        WorkflowIntent::CreateJoint {
            target,
            participant_a,
            participant_b,
            volume_min,
            volume_max,
        } => {
            let snapshot = store.current();
            if snapshot.joint(target).is_some() {
                return Err(crate::document::CanonicalError::JointAlreadyExists(target).into());
            }
            if snapshot.resolve_slot(&participant_a) != SlotResolution::Resolved
                || snapshot.resolve_slot(&participant_b) != SlotResolution::Resolved
            {
                return Err(crate::document::CanonicalError::UnresolvedDerivedOutput.into());
            }
            (
                ProposalGoal::CreateJoint(target),
                AuthoritativeDependency::Joint(target),
                CanonicalCommand::UpsertJoint(
                    crate::prismatic::CanonicalJoint::new(
                        target,
                        participant_a,
                        participant_b,
                        crate::prismatic::Aabb::new(volume_min, volume_max)
                            .map_err(crate::document::CanonicalError::from)?,
                    )
                    .map_err(crate::document::CanonicalError::from)?,
                ),
            )
        }
        WorkflowIntent::DeleteJoint { target } => (
            ProposalGoal::DeleteJoint(target),
            AuthoritativeDependency::Joint(target),
            CanonicalCommand::DeleteJoint { id: target },
        ),
        WorkflowIntent::DeleteSpace { target } => (
            ProposalGoal::DeleteSpace(target),
            AuthoritativeDependency::Space(target),
            CanonicalCommand::DeleteSpace { id: target },
        ),
        WorkflowIntent::DeleteClearanceVolume { target } => (
            ProposalGoal::DeleteClearanceVolume(target),
            AuthoritativeDependency::ClearanceVolume(target),
            CanonicalCommand::DeleteClearanceVolume { id: target },
        ),
        WorkflowIntent::DeletePersistentDimension { target } => (
            ProposalGoal::DeletePersistentDimension(target),
            AuthoritativeDependency::PersistentDimension(target),
            CanonicalCommand::DeletePersistentDimension { id: target },
        ),
        WorkflowIntent::RecomputeFeatureParameter { target } => {
            let snapshot = store.current();
            let mut bindings = snapshot.feature_parameter_bindings();
            let Some(binding) = bindings.next() else {
                return Err(
                    crate::document::CanonicalError::FeatureParameterBindingNotFound(target).into(),
                );
            };
            if binding.target != target || bindings.next().is_some() {
                return Err(
                    crate::document::CanonicalError::InvalidFeatureParameterBinding(target).into(),
                );
            }
            (
                ProposalGoal::RecomputeFeatureParameter(target),
                AuthoritativeDependency::Feature(target.feature_id),
                CanonicalCommand::RecomputeFeatureParameters {
                    identity: EvaluationIdentity::default(),
                },
            )
        }
        WorkflowIntent::SetRuleDimension { target, value_text } => {
            let authority = AuthoritativeDependency::EvaluatorNode(target);
            (
                ProposalGoal::SetRuleDimension(target),
                authority,
                CanonicalCommand::SetEvaluatorDimension {
                    id: target,
                    dimension: Dimension::from_decimal(value_text)?,
                },
            )
        }
        WorkflowIntent::RenameEvaluatorNode { target, name } => (
            ProposalGoal::RenameEvaluatorNode(target),
            AuthoritativeDependency::EvaluatorNode(target),
            CanonicalCommand::RenameEvaluatorNode { id: target, name },
        ),
        WorkflowIntent::SetEvaluatorExpression { target, expression } => (
            ProposalGoal::SetEvaluatorExpression(target),
            AuthoritativeDependency::EvaluatorNode(target),
            CanonicalCommand::SetNodeExpression {
                id: target,
                expression,
            },
        ),
        WorkflowIntent::SetRuleOutputs { target, outputs } => (
            ProposalGoal::SetRuleOutputs(target),
            AuthoritativeDependency::EvaluatorNode(target),
            CanonicalCommand::SetRuleOutputs {
                id: target,
                outputs,
            },
        ),
        WorkflowIntent::SetFeatureDimension { target, value_text } => {
            let authority = AuthoritativeDependency::Feature(target);
            (
                ProposalGoal::SetFeatureDimension(target),
                authority,
                CanonicalCommand::SetFeatureDimension {
                    id: target,
                    dimension: Dimension::from_decimal(value_text)?,
                },
            )
        }
        WorkflowIntent::SetBottleControlDimension {
            target,
            control,
            value_text,
        } => {
            let authority = AuthoritativeDependency::Feature(target);
            (
                ProposalGoal::SetBottleControlDimension(target, control),
                authority,
                CanonicalCommand::SetBottleControlDimension {
                    id: target,
                    control,
                    dimension: Dimension::from_decimal(value_text)?,
                },
            )
        }
        WorkflowIntent::SetBottleEdgeFinishKind { target, kind } => {
            let authority = AuthoritativeDependency::Feature(target);
            (
                ProposalGoal::SetBottleEdgeFinishKind(target),
                authority,
                CanonicalCommand::SetBottleEdgeFinishKind { id: target, kind },
            )
        }
        WorkflowIntent::SetProfilePoints { target, points_mm } => {
            let authority = AuthoritativeDependency::Feature(target);
            (
                ProposalGoal::SetProfilePoints(target),
                authority,
                CanonicalCommand::SetProfilePoints {
                    id: target,
                    points_mm,
                },
            )
        }
        WorkflowIntent::RenameDefinition { target, name } => {
            let authority = AuthoritativeDependency::Definition(target);
            (
                ProposalGoal::RenameDefinition(target),
                authority,
                CanonicalCommand::RenameDefinition { id: target, name },
            )
        }
        WorkflowIntent::SetOccurrenceVisibility { target, visible } => {
            let authority = AuthoritativeDependency::Occurrence(target);
            (
                ProposalGoal::SetOccurrenceVisibility(target),
                authority,
                CanonicalCommand::SetOccurrenceVisibility {
                    id: target,
                    visible,
                },
            )
        }
        WorkflowIntent::SetOccurrenceTag { target, tag } => {
            let authority = AuthoritativeDependency::Occurrence(target);
            (
                ProposalGoal::SetOccurrenceTag(target),
                authority,
                CanonicalCommand::SetOccurrenceTag { id: target, tag },
            )
        }
        WorkflowIntent::SetTagVisibility { target, visible } => {
            let authority = AuthoritativeDependency::Tag(target);
            (
                ProposalGoal::SetTagVisibility(target),
                authority,
                CanonicalCommand::SetTagVisibility {
                    id: target,
                    visible,
                },
            )
        }
        WorkflowIntent::RepointOccurrence { target, definition } => {
            let authority = AuthoritativeDependency::Occurrence(target);
            (
                ProposalGoal::RepointOccurrence(target),
                authority,
                CanonicalCommand::RepointOccurrence {
                    id: target,
                    definition_id: definition,
                },
            )
        }
        WorkflowIntent::SetOccurrenceParent { target, parent } => {
            let authority = AuthoritativeDependency::Occurrence(target);
            (
                ProposalGoal::SetOccurrenceParent(target),
                authority,
                CanonicalCommand::SetOccurrenceParent { id: target, parent },
            )
        }
        WorkflowIntent::SetGroupParent { target, parent } => (
            ProposalGoal::SetGroupParent(target),
            AuthoritativeDependency::Group(target),
            CanonicalCommand::SetGroupParent { id: target, parent },
        ),
        WorkflowIntent::SetCollectionOccurrences {
            target,
            occurrence_ids,
        } => (
            ProposalGoal::SetCollectionOccurrences(target),
            AuthoritativeDependency::Collection(target),
            CanonicalCommand::SetCollectionOccurrences {
                id: target,
                occurrence_ids,
            },
        ),
        WorkflowIntent::CreateTag {
            target,
            name,
            visible,
        } => (
            ProposalGoal::CreateTag(target),
            AuthoritativeDependency::Tag(target),
            CanonicalCommand::CreateTag {
                id: target,
                name,
                visible,
            },
        ),
        WorkflowIntent::DeleteTag { target } => (
            ProposalGoal::DeleteTag(target),
            AuthoritativeDependency::Tag(target),
            CanonicalCommand::DeleteTag { id: target },
        ),
        WorkflowIntent::CreateCollection { target, name } => (
            ProposalGoal::CreateCollection(target),
            AuthoritativeDependency::Collection(target),
            CanonicalCommand::CreateCollection { id: target, name },
        ),
        WorkflowIntent::DeleteCollection { target } => (
            ProposalGoal::DeleteCollection(target),
            AuthoritativeDependency::Collection(target),
            CanonicalCommand::DeleteCollection { id: target },
        ),
        WorkflowIntent::DeleteGroup { target } => (
            ProposalGoal::DeleteGroup(target),
            AuthoritativeDependency::Group(target),
            CanonicalCommand::DeleteGroup { id: target },
        ),
        WorkflowIntent::DeleteOccurrence { target } => (
            ProposalGoal::DeleteOccurrence(target),
            AuthoritativeDependency::Occurrence(target),
            CanonicalCommand::DeleteOccurrence { id: target },
        ),
        WorkflowIntent::CreateDefinition { target, name } => (
            ProposalGoal::CreateDefinition(target),
            AuthoritativeDependency::Definition(target),
            CanonicalCommand::CreateDefinition { id: target, name },
        ),
        WorkflowIntent::DeleteDefinition { target } => {
            if store
                .current()
                .definition(target)
                .is_some_and(|definition| {
                    !definition.feature_ids().is_empty()
                        || !definition.local_occurrence_ids().is_empty()
                        || !definition.local_group_ids().is_empty()
                })
            {
                return Err(crate::document::CanonicalError::DefinitionNotEmpty(target).into());
            }
            (
                ProposalGoal::DeleteDefinition(target),
                AuthoritativeDependency::Definition(target),
                CanonicalCommand::DeleteDefinition { id: target },
            )
        }
        WorkflowIntent::CreateProfileFeature {
            target,
            definition,
            name,
            points_mm,
        } => (
            ProposalGoal::CreateProfileFeature(target),
            AuthoritativeDependency::Feature(target),
            CanonicalCommand::CreateFeature {
                id: target,
                definition_id: definition,
                name,
                kind: FeatureKind::Profile { points_mm },
            },
        ),
        WorkflowIntent::DeleteProfileFeature { target } => {
            if store
                .current()
                .feature(target)
                .is_some_and(|feature| !matches!(feature.kind(), FeatureKind::Profile { .. }))
            {
                return Err(crate::document::CanonicalError::FeatureIsNotProfile(target).into());
            }
            (
                ProposalGoal::DeleteProfileFeature(target),
                AuthoritativeDependency::Feature(target),
                CanonicalCommand::DeleteFeature { id: target },
            )
        }
        WorkflowIntent::CreateGroup { target, name } => (
            ProposalGoal::CreateGroup(target),
            AuthoritativeDependency::Group(target),
            CanonicalCommand::CreateGroup {
                id: target,
                name,
                transform: Transform::identity(),
                parent: None,
            },
        ),
        WorkflowIntent::CreateOccurrence {
            target,
            definition,
            name,
        } => (
            ProposalGoal::CreateOccurrence(target),
            AuthoritativeDependency::Occurrence(target),
            CanonicalCommand::CreateOccurrence {
                id: target,
                definition_id: definition,
                name,
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ),
        WorkflowIntent::CloneProfileDefinitionAndRepoint {
            target,
            source_definition,
            source_feature,
            new_definition,
            new_feature,
            name,
        } => {
            let snapshot = store.current();
            let occurrence = snapshot
                .occurrence(target)
                .ok_or(crate::document::CanonicalError::OccurrenceNotFound(target))?;
            if occurrence.definition_id() != source_definition {
                return Err(crate::document::CanonicalError::OccurrenceDefinitionMismatch.into());
            }
            let definition = snapshot.definition(source_definition).ok_or(
                crate::document::CanonicalError::DefinitionNotFound(source_definition),
            )?;
            if definition.feature_ids() != [source_feature]
                || !definition.local_occurrence_ids().is_empty()
                || !definition.local_group_ids().is_empty()
                || snapshot
                    .feature_parameter_bindings()
                    .any(|binding| binding.target.feature_id == source_feature)
            {
                return Err(crate::document::CanonicalError::InvalidFeatureMap.into());
            }
            let feature = snapshot.feature(source_feature).ok_or(
                crate::document::CanonicalError::FeatureNotFound(source_feature),
            )?;
            if !matches!(feature.kind(), FeatureKind::Profile { .. }) {
                return Err(
                    crate::document::CanonicalError::FeatureIsNotProfile(source_feature).into(),
                );
            }
            if snapshot.definition(new_definition).is_some() {
                return Err(crate::document::CanonicalError::DefinitionAlreadyExists(
                    new_definition,
                )
                .into());
            }
            if snapshot.feature(new_feature).is_some() {
                return Err(
                    crate::document::CanonicalError::FeatureAlreadyExists(new_feature).into(),
                );
            }
            (
                ProposalGoal::CloneProfileDefinitionAndRepoint(target),
                AuthoritativeDependency::Occurrence(target),
                CanonicalCommand::CloneDefinitionAndRepoint(CloneDefinitionPlan::new(
                    target,
                    source_definition,
                    new_definition,
                    name,
                    vec![(source_feature, new_feature)],
                )),
            )
        }
        WorkflowIntent::ConvertEmptyGroupToComponent {
            target,
            new_definition,
            new_occurrence,
            name,
        } => {
            let snapshot = store.current();
            snapshot
                .group(target)
                .ok_or(crate::document::CanonicalError::GroupNotFound(target))?;
            if snapshot
                .groups()
                .any(|group| group.parent() == Some(target))
                || snapshot
                    .occurrences()
                    .any(|occurrence| occurrence.parent() == Some(target))
            {
                return Err(crate::document::CanonicalError::InvalidLocalGraph.into());
            }
            if snapshot.definition(new_definition).is_some() {
                return Err(crate::document::CanonicalError::DefinitionAlreadyExists(
                    new_definition,
                )
                .into());
            }
            if snapshot.occurrence(new_occurrence).is_some() {
                return Err(crate::document::CanonicalError::OccurrenceAlreadyExists(
                    new_occurrence,
                )
                .into());
            }
            (
                ProposalGoal::ConvertEmptyGroupToComponent(target),
                AuthoritativeDependency::GroupSubtree(target),
                CanonicalCommand::ConvertGroupToComponent(ConvertGroupPlan::new(
                    target,
                    new_definition,
                    new_occurrence,
                    name,
                )),
            )
        }
        WorkflowIntent::SetGroupTranslation {
            target,
            x_mm_text,
            y_mm_text,
            z_mm_text,
        } => {
            let parse = |value: String| {
                value
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| crate::document::CanonicalError::InvalidTransform)
            };
            let x_mm = parse(x_mm_text)?;
            let y_mm = parse(y_mm_text)?;
            let z_mm = parse(z_mm_text)?;
            let snapshot = store.current();
            let mut matrix = *snapshot
                .group(target)
                .ok_or(crate::document::CanonicalError::GroupNotFound(target))?
                .transform()
                .matrix();
            matrix[3] = x_mm;
            matrix[7] = y_mm;
            matrix[11] = z_mm;
            (
                ProposalGoal::SetGroupTranslation(target),
                AuthoritativeDependency::Group(target),
                CanonicalCommand::SetGroupTransform {
                    id: target,
                    transform: Transform::from_matrix(matrix)?,
                },
            )
        }
        WorkflowIntent::AtomicMultiCommandEdit { .. } => unreachable!(),
        WorkflowIntent::SetOccurrenceTranslation {
            target,
            x_mm_text,
            y_mm_text,
            z_mm_text,
        } => {
            let authority = AuthoritativeDependency::Occurrence(target);
            let parse = |value: String| {
                value
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| crate::document::CanonicalError::InvalidTransform)
            };
            let x_mm = parse(x_mm_text)?;
            let y_mm = parse(y_mm_text)?;
            let z_mm = parse(z_mm_text)?;
            let snapshot = store.current();
            let mut matrix = *snapshot
                .occurrence(target)
                .ok_or(crate::document::CanonicalError::OccurrenceNotFound(target))?
                .transform()
                .matrix();
            matrix[3] = x_mm;
            matrix[7] = y_mm;
            matrix[11] = z_mm;
            (
                ProposalGoal::SetOccurrenceTranslation(target),
                authority,
                CanonicalCommand::SetOccurrenceTransform {
                    id: target,
                    transform: Transform::from_matrix(matrix)?,
                },
            )
        }
    };

    let mut assumptions = if matches!(
        required,
        IntentCapability::CreateEvaluatorInput
            | IntentCapability::CreateEvaluatorExpression
            | IntentCapability::CreateEvaluatorRule
            | IntentCapability::CreateRuleOverride
            | IntentCapability::CreateFeatureParameterBinding
            | IntentCapability::CreatePersistentDimension
            | IntentCapability::CreateSpace
            | IntentCapability::CreateClearanceVolume
            | IntentCapability::CreateJoint
            | IntentCapability::CreateTag
            | IntentCapability::CreateCollection
            | IntentCapability::CreateDefinition
            | IntentCapability::CreateProfileFeature
            | IntentCapability::CreateGroup
            | IntentCapability::CreateOccurrence
    ) {
        vec![ProposalAssumption::TargetMissing(target)]
    } else if matches!(
        required,
        IntentCapability::RenameEvaluatorNode
            | IntentCapability::SetEvaluatorExpression
            | IntentCapability::SetRuleOutputs
            | IntentCapability::SetBottleControlDimension
            | IntentCapability::SetBottleEdgeFinishKind
            | IntentCapability::SetProfilePoints
            | IntentCapability::RenameDefinition
            | IntentCapability::SetOccurrenceVisibility
            | IntentCapability::SetOccurrenceTranslation
            | IntentCapability::SetOccurrenceTag
            | IntentCapability::SetTagVisibility
            | IntentCapability::RepointOccurrence
            | IntentCapability::SetOccurrenceParent
            | IntentCapability::SetGroupTranslation
            | IntentCapability::SetGroupParent
            | IntentCapability::SetCollectionOccurrences
            | IntentCapability::DeleteRuleOverride
            | IntentCapability::DeleteFeatureParameterBinding
            | IntentCapability::RecomputeFeatureParameter
            | IntentCapability::DeleteJoint
            | IntentCapability::DeleteSpace
            | IntentCapability::DeleteClearanceVolume
            | IntentCapability::DeletePersistentDimension
            | IntentCapability::DeleteTag
            | IntentCapability::DeleteCollection
            | IntentCapability::DeleteGroup
            | IntentCapability::DeleteOccurrence
            | IntentCapability::DeleteDefinition
            | IntentCapability::DeleteProfileFeature
            | IntentCapability::CloneProfileDefinitionAndRepoint
            | IntentCapability::ConvertEmptyGroupToComponent
    ) {
        vec![ProposalAssumption::TargetExists(target)]
    } else {
        vec![
            ProposalAssumption::TargetExists(target),
            ProposalAssumption::TargetHasDimension(target),
        ]
    };
    assumptions.extend(
        requested_secondary_targets
            .into_iter()
            .map(ProposalAssumption::TargetExists),
    );
    assumptions.extend(
        requested_missing_targets
            .into_iter()
            .map(ProposalAssumption::TargetMissing),
    );

    store
        .prepare_proposal_with_context(
            CommandBatch::new(vec![command]),
            ProposalContext {
                principal: match request.grant.principal {
                    RequestingPrincipal::LocalAssistant => ProposalPrincipal::LocalAssistant,
                    RequestingPrincipal::Plugin(id) => ProposalPrincipal::Plugin(id),
                },
                goal,
                assumptions,
                risk: ProposalRisk::Standard,
                confirmation: ProposalConfirmation::ReviewRequired,
                requested_budget: request.requested_budget,
            },
        )
        .map_err(IntentError::Proposal)
}

#[derive(Debug, PartialEq)]
pub enum IntentError {
    CapabilityDenied(IntentCapability),
    Canonical(crate::document::CanonicalError),
    Proposal(ProposalPrepareError),
}

impl fmt::Display for IntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapabilityDenied(capability) => {
                write!(
                    formatter,
                    "intent capability {capability:?} was not granted"
                )
            }
            Self::Canonical(error) => error.fmt(formatter),
            Self::Proposal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for IntentError {}

impl From<crate::document::CanonicalError> for IntentError {
    fn from(error: crate::document::CanonicalError) -> Self {
        Self::Canonical(error)
    }
}
