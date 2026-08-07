use crate::document::{
    AuthoritativeDependency, CanonicalCommand, CommandBatch, Dimension, DocumentStore, FeatureId,
    NodeId, Proposal, ProposalAssumption, ProposalBudget, ProposalConfirmation, ProposalContext,
    ProposalGoal, ProposalPrepareError, ProposalPrincipal, ProposalRisk,
};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntentCapability {
    SetRuleDimension,
    SetFeatureDimension,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowIntent {
    SetRuleDimension {
        target: NodeId,
        value_text: String,
    },
    SetFeatureDimension {
        target: FeatureId,
        value_text: String,
    },
}

impl WorkflowIntent {
    const fn required_capability(&self) -> IntentCapability {
        match self {
            Self::SetRuleDimension { .. } => IntentCapability::SetRuleDimension,
            Self::SetFeatureDimension { .. } => IntentCapability::SetFeatureDimension,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentRequest {
    pub grant: IntentGrant,
    pub intent: WorkflowIntent,
    pub requested_budget: ProposalBudget,
}

impl IntentRequest {
    #[must_use]
    pub fn m7a(intent: WorkflowIntent) -> Self {
        Self {
            grant: IntentGrant::m7a_local_assistant(),
            intent,
            requested_budget: ProposalBudget::M7A_SINGLE_CHANGE,
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
    };

    store
        .prepare_proposal_with_context(
            CommandBatch::new(vec![command]),
            ProposalContext {
                principal: match request.grant.principal {
                    RequestingPrincipal::LocalAssistant => ProposalPrincipal::LocalAssistant,
                    RequestingPrincipal::Plugin(id) => ProposalPrincipal::Plugin(id),
                },
                goal,
                assumptions: vec![
                    ProposalAssumption::TargetExists(target),
                    ProposalAssumption::TargetHasDimension(target),
                ],
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
