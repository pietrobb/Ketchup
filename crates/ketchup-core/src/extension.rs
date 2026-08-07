use crate::document::{DocumentStore, FeatureId, NodeId, Proposal, ProposalBudget};
use crate::intent::{
    IntentCapability, IntentError, IntentGrant, IntentRequest, WorkflowIntent, propose_intent,
};
use crate::state_view::encode_semantic_state;
use std::collections::BTreeSet;
use std::fmt;

pub const PLUGIN_PROTOCOL_V1: &str = "ketchup.plugin.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PluginCapability {
    QueryAgentState,
    SetRuleDimension,
    SetFeatureDimension,
}

impl PluginCapability {
    #[must_use]
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::QueryAgentState => "query.agent-state.v1",
            Self::SetRuleDimension => "intent.set-rule-dimension.v1",
            Self::SetFeatureDimension => "intent.set-feature-dimension.v1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginLimits {
    pub max_requests: usize,
    pub max_query_bytes: usize,
    pub proposal_budget: ProposalBudget,
}

impl PluginLimits {
    pub const HOST_MAX: Self = Self {
        max_requests: 8,
        max_query_bytes: 64 * 1024,
        proposal_budget: ProposalBudget::HOST_MAX,
    };

    pub const M7B_PILOT: Self = Self {
        max_requests: 4,
        max_query_bytes: 32 * 1024,
        proposal_budget: ProposalBudget::M7A_SINGLE_CHANGE,
    };

    fn within(self, maximum: Self) -> bool {
        self.max_requests <= maximum.max_requests
            && self.max_query_bytes <= maximum.max_query_bytes
            && self.proposal_budget.max_commands <= maximum.proposal_budget.max_commands
            && self.proposal_budget.max_read_dependencies
                <= maximum.proposal_budget.max_read_dependencies
            && self.proposal_budget.max_write_targets <= maximum.proposal_budget.max_write_targets
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginManifest {
    protocol: String,
    package: String,
    version: String,
    principal_id: u64,
    capabilities: BTreeSet<PluginCapability>,
    limits: PluginLimits,
}

impl PluginManifest {
    pub fn new(
        package: impl Into<String>,
        version: impl Into<String>,
        principal_id: u64,
        capabilities: impl IntoIterator<Item = PluginCapability>,
        limits: PluginLimits,
    ) -> Result<Self, PluginGatewayError> {
        let package = package.into();
        let version = version.into();
        if package.is_empty()
            || package.len() > 128
            || !package
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(PluginGatewayError::InvalidManifest(
                "package must be 1..128 ASCII identifier bytes".to_owned(),
            ));
        }
        if version.is_empty()
            || version.len() > 64
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(PluginGatewayError::InvalidManifest(
                "version must be 1..64 ASCII identifier bytes".to_owned(),
            ));
        }
        if principal_id == 0 {
            return Err(PluginGatewayError::InvalidManifest(
                "principal id must be non-zero".to_owned(),
            ));
        }
        if !limits.within(PluginLimits::HOST_MAX) {
            return Err(PluginGatewayError::HostLimitExceeded);
        }
        Ok(Self {
            protocol: PLUGIN_PROTOCOL_V1.to_owned(),
            package,
            version,
            principal_id,
            capabilities: capabilities.into_iter().collect(),
            limits,
        })
    }

    #[must_use]
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn principal_id(&self) -> u64 {
        self.principal_id
    }

    #[must_use]
    pub const fn capabilities(&self) -> &BTreeSet<PluginCapability> {
        &self.capabilities
    }

    #[must_use]
    pub const fn limits(&self) -> PluginLimits {
        self.limits
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginGrant {
    principal_id: u64,
    capabilities: BTreeSet<PluginCapability>,
    limits: PluginLimits,
}

impl PluginGrant {
    #[must_use]
    pub fn new(
        principal_id: u64,
        capabilities: impl IntoIterator<Item = PluginCapability>,
        limits: PluginLimits,
    ) -> Self {
        Self {
            principal_id,
            capabilities: capabilities.into_iter().collect(),
            limits,
        }
    }

    #[must_use]
    pub const fn principal_id(&self) -> u64 {
        self.principal_id
    }

    #[must_use]
    pub const fn capabilities(&self) -> &BTreeSet<PluginCapability> {
        &self.capabilities
    }

    #[must_use]
    pub const fn limits(&self) -> PluginLimits {
        self.limits
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginRequest {
    QueryAgentState,
    SetRuleDimension {
        target: NodeId,
        value_text: String,
    },
    SetFeatureDimension {
        target: FeatureId,
        value_text: String,
    },
}

impl PluginRequest {
    const fn required_capability(&self) -> PluginCapability {
        match self {
            Self::QueryAgentState => PluginCapability::QueryAgentState,
            Self::SetRuleDimension { .. } => PluginCapability::SetRuleDimension,
            Self::SetFeatureDimension { .. } => PluginCapability::SetFeatureDimension,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginResponse {
    AgentState(String),
    Proposal(Box<Proposal>),
}

pub struct PluginGateway {
    manifest: PluginManifest,
    grant: PluginGrant,
    request_count: usize,
    query_bytes: usize,
}

impl PluginGateway {
    pub fn new(manifest: PluginManifest, grant: PluginGrant) -> Result<Self, PluginGatewayError> {
        if manifest.principal_id != grant.principal_id {
            return Err(PluginGatewayError::PrincipalMismatch);
        }
        if !grant.limits.within(PluginLimits::HOST_MAX) {
            return Err(PluginGatewayError::HostLimitExceeded);
        }
        Ok(Self {
            manifest,
            grant,
            request_count: 0,
            query_bytes: 0,
        })
    }

    #[must_use]
    pub const fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    pub fn handle(
        &mut self,
        store: &DocumentStore,
        request: PluginRequest,
    ) -> Result<PluginResponse, PluginGatewayError> {
        self.request_count = self.request_count.saturating_add(1);
        let effective_max_requests = self
            .manifest
            .limits
            .max_requests
            .min(self.grant.limits.max_requests);
        if self.request_count > effective_max_requests {
            return Err(PluginGatewayError::RequestBudgetExceeded);
        }

        let required = request.required_capability();
        if !self.manifest.capabilities.contains(&required)
            || !self.grant.capabilities.contains(&required)
        {
            return Err(PluginGatewayError::CapabilityDenied(required));
        }

        match request {
            PluginRequest::QueryAgentState => {
                let state = encode_semantic_state(&store.current()).agent_v1();
                let effective_max_bytes = self
                    .manifest
                    .limits
                    .max_query_bytes
                    .min(self.grant.limits.max_query_bytes);
                let next_bytes = self.query_bytes.saturating_add(state.len());
                if next_bytes > effective_max_bytes {
                    return Err(PluginGatewayError::QueryBudgetExceeded {
                        attempted_bytes: next_bytes,
                        max_bytes: effective_max_bytes,
                    });
                }
                self.query_bytes = next_bytes;
                Ok(PluginResponse::AgentState(state))
            }
            PluginRequest::SetRuleDimension { target, value_text } => self.propose(
                store,
                IntentCapability::SetRuleDimension,
                WorkflowIntent::SetRuleDimension { target, value_text },
            ),
            PluginRequest::SetFeatureDimension { target, value_text } => self.propose(
                store,
                IntentCapability::SetFeatureDimension,
                WorkflowIntent::SetFeatureDimension { target, value_text },
            ),
        }
    }

    fn propose(
        &self,
        store: &DocumentStore,
        intent_capability: IntentCapability,
        intent: WorkflowIntent,
    ) -> Result<PluginResponse, PluginGatewayError> {
        let requested_budget = PluginLimits {
            max_requests: self
                .manifest
                .limits
                .max_requests
                .min(self.grant.limits.max_requests),
            max_query_bytes: self
                .manifest
                .limits
                .max_query_bytes
                .min(self.grant.limits.max_query_bytes),
            proposal_budget: ProposalBudget {
                max_commands: self
                    .manifest
                    .limits
                    .proposal_budget
                    .max_commands
                    .min(self.grant.limits.proposal_budget.max_commands),
                max_read_dependencies: self
                    .manifest
                    .limits
                    .proposal_budget
                    .max_read_dependencies
                    .min(self.grant.limits.proposal_budget.max_read_dependencies),
                max_write_targets: self
                    .manifest
                    .limits
                    .proposal_budget
                    .max_write_targets
                    .min(self.grant.limits.proposal_budget.max_write_targets),
            },
        }
        .proposal_budget;
        let request = IntentRequest {
            grant: IntentGrant::m7b_plugin(self.manifest.principal_id, [intent_capability]),
            intent,
            requested_budget,
        };
        propose_intent(store, request)
            .map(|proposal| PluginResponse::Proposal(Box::new(proposal)))
            .map_err(PluginGatewayError::Intent)
    }
}

#[derive(Debug, PartialEq)]
pub enum PluginGatewayError {
    InvalidManifest(String),
    PrincipalMismatch,
    HostLimitExceeded,
    CapabilityDenied(PluginCapability),
    RequestBudgetExceeded,
    QueryBudgetExceeded {
        attempted_bytes: usize,
        max_bytes: usize,
    },
    Intent(IntentError),
}

impl fmt::Display for PluginGatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(message) => {
                write!(formatter, "invalid plugin manifest: {message}")
            }
            Self::PrincipalMismatch => {
                formatter.write_str("plugin manifest and grant principals differ")
            }
            Self::HostLimitExceeded => {
                formatter.write_str("plugin limits exceed the host envelope")
            }
            Self::CapabilityDenied(capability) => {
                write!(
                    formatter,
                    "plugin capability {} was not granted",
                    capability.protocol_name()
                )
            }
            Self::RequestBudgetExceeded => {
                formatter.write_str("plugin request budget was exceeded")
            }
            Self::QueryBudgetExceeded {
                attempted_bytes,
                max_bytes,
            } => write!(
                formatter,
                "plugin query budget exceeded: {attempted_bytes} bytes requested, {max_bytes} allowed"
            ),
            Self::Intent(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PluginGatewayError {}
