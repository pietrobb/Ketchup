use ketchup_model::document::{
    AuthoritativeDependency, CanonicalCommand, CommandBatch, ConvertGroupPlan, DefinitionId,
    Dimension, DocumentStore, FeatureId, GroupId, NodeId, OccurrenceId, Proposal,
    ProposalAssumption, ProposalBudget, ProposalConfirmation, ProposalContext, ProposalGoal,
    ProposalPrepareError, ProposalPrincipal, ProposalRisk, TagId, Transform,
};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntentCapability {
    SetRuleDimension,
    SetFeatureDimension,
    SetProfilePoints,
    RenameDefinition,
    SetOccurrenceVisibility,
    RepointOccurrence,
    SetOccurrenceParent,
    SetGroupTranslation,
    SetGroupParent,
    DeleteTag,
    DeleteGroup,
    DeleteDefinition,
    CreateGroup,
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
                IntentCapability::SetRuleDimension,
                IntentCapability::SetFeatureDimension,
                IntentCapability::SetProfilePoints,
                IntentCapability::RenameDefinition,
                IntentCapability::SetOccurrenceVisibility,
                IntentCapability::RepointOccurrence,
                IntentCapability::SetOccurrenceParent,
                IntentCapability::SetGroupTranslation,
                IntentCapability::SetGroupParent,
                IntentCapability::DeleteTag,
                IntentCapability::DeleteGroup,
                IntentCapability::DeleteDefinition,
                IntentCapability::CreateGroup,
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
    SetRuleDimension {
        target: NodeId,
        value_text: String,
    },
    SetFeatureDimension {
        target: FeatureId,
        value_text: String,
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
    DeleteTag {
        target: TagId,
    },
    DeleteGroup {
        target: GroupId,
    },
    DeleteDefinition {
        target: DefinitionId,
    },
    CreateGroup {
        target: GroupId,
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
            Self::SetRuleDimension { .. } => IntentCapability::SetRuleDimension,
            Self::SetFeatureDimension { .. } => IntentCapability::SetFeatureDimension,
            Self::SetProfilePoints { .. } => IntentCapability::SetProfilePoints,
            Self::RenameDefinition { .. } => IntentCapability::RenameDefinition,
            Self::SetOccurrenceVisibility { .. } => IntentCapability::SetOccurrenceVisibility,
            Self::RepointOccurrence { .. } => IntentCapability::RepointOccurrence,
            Self::SetOccurrenceParent { .. } => IntentCapability::SetOccurrenceParent,
            Self::SetGroupTranslation { .. } => IntentCapability::SetGroupTranslation,
            Self::SetGroupParent { .. } => IntentCapability::SetGroupParent,
            Self::DeleteTag { .. } => IntentCapability::DeleteTag,
            Self::DeleteGroup { .. } => IntentCapability::DeleteGroup,
            Self::DeleteDefinition { .. } => IntentCapability::DeleteDefinition,
            Self::CreateGroup { .. } => IntentCapability::CreateGroup,
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
            WorkflowIntent::ConvertEmptyGroupToComponent { .. } => {
                ProposalBudget::M18C_CLONE_PROFILE_DEFINITION
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
    let requested_secondary_targets = match &request.intent {
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
        _ => Vec::new(),
    };
    let requested_missing_targets = match &request.intent {
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
        WorkflowIntent::DeleteTag { target } => (
            ProposalGoal::DeleteTag(target),
            AuthoritativeDependency::Tag(target),
            CanonicalCommand::DeleteTag { id: target },
        ),
        WorkflowIntent::DeleteGroup { target } => (
            ProposalGoal::DeleteGroup(target),
            AuthoritativeDependency::Group(target),
            CanonicalCommand::DeleteGroup { id: target },
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
                return Err(
                    ketchup_model::document::CanonicalError::DefinitionNotEmpty(target).into(),
                );
            }
            (
                ProposalGoal::DeleteDefinition(target),
                AuthoritativeDependency::Definition(target),
                CanonicalCommand::DeleteDefinition { id: target },
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
        WorkflowIntent::ConvertEmptyGroupToComponent {
            target,
            new_definition,
            new_occurrence,
            name,
        } => {
            let snapshot = store.current();
            snapshot.group(target).ok_or(
                ketchup_model::document::CanonicalError::GroupNotFound(target),
            )?;
            if snapshot
                .groups()
                .any(|group| group.parent() == Some(target))
                || snapshot
                    .occurrences()
                    .any(|occurrence| occurrence.parent() == Some(target))
            {
                return Err(ketchup_model::document::CanonicalError::InvalidLocalGraph.into());
            }
            if snapshot.definition(new_definition).is_some() {
                return Err(
                    ketchup_model::document::CanonicalError::DefinitionAlreadyExists(
                        new_definition,
                    )
                    .into(),
                );
            }
            if snapshot.occurrence(new_occurrence).is_some() {
                return Err(
                    ketchup_model::document::CanonicalError::OccurrenceAlreadyExists(
                        new_occurrence,
                    )
                    .into(),
                );
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
                    .map_err(|_: std::num::ParseFloatError| {
                        ketchup_model::document::CanonicalError::InvalidTransform
                    })
            };
            let x_mm = parse(x_mm_text)?;
            let y_mm = parse(y_mm_text)?;
            let z_mm = parse(z_mm_text)?;
            let snapshot = store.current();
            let mut matrix = *snapshot
                .group(target)
                .ok_or(ketchup_model::document::CanonicalError::GroupNotFound(
                    target,
                ))?
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
    };

    let mut assumptions = if matches!(required, IntentCapability::CreateGroup) {
        vec![ProposalAssumption::TargetMissing(target)]
    } else if matches!(
        required,
        IntentCapability::SetProfilePoints
            | IntentCapability::RenameDefinition
            | IntentCapability::SetOccurrenceVisibility
            | IntentCapability::RepointOccurrence
            | IntentCapability::SetOccurrenceParent
            | IntentCapability::SetGroupTranslation
            | IntentCapability::SetGroupParent
            | IntentCapability::DeleteTag
            | IntentCapability::DeleteGroup
            | IntentCapability::DeleteDefinition
            | IntentCapability::ConvertEmptyGroupToComponent
    ) {
        vec![ProposalAssumption::TargetExists(target)]
    } else {
        vec![
            ProposalAssumption::TargetExists(target.clone()),
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
    Canonical(ketchup_model::document::CanonicalError),
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

impl From<ketchup_geometry::dimension::DimensionError> for IntentError {
    fn from(error: ketchup_geometry::dimension::DimensionError) -> Self {
        Self::Canonical(error.into())
    }
}

impl From<ketchup_model::document::CanonicalError> for IntentError {
    fn from(error: ketchup_model::document::CanonicalError) -> Self {
        Self::Canonical(error)
    }
}
