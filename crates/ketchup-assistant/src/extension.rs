//! The plugin gateway: what a plugin may ask the host for. A plugin edits the
//! model only with the typed CAD program MCP uses; the gateway checks the
//! capability, the operation allowlist and the budgets and hands the program
//! back, and the caller plans and commits it on the same path as MCP.

use crate::sidecar::{AssistantCadEditOperation, AssistantCadEditProgram};
use ketchup_model::document::Snapshot;
use ketchup_model::state_view::encode_semantic_state;
use std::collections::BTreeSet;
use std::fmt;

pub const PLUGIN_PROTOCOL_V1: &str = "ketchup.plugin.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PluginCapability {
    QueryAgentState,
    EditCadProgram,
}

impl PluginCapability {
    #[must_use]
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::QueryAgentState => "query.agent-state.v1",
            Self::EditCadProgram => "edit.cad-program.v1",
        }
    }

    #[must_use]
    pub fn from_protocol_name(name: &str) -> Option<Self> {
        [Self::QueryAgentState, Self::EditCadProgram]
            .into_iter()
            .find(|capability| capability.protocol_name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginLimits {
    pub max_requests: usize,
    pub max_query_bytes: usize,
    /// Operations one `Apply` program may carry.
    pub max_operations: usize,
}

impl PluginLimits {
    pub const HOST_MAX: Self = Self {
        max_requests: 8,
        max_query_bytes: 64 * 1024,
        max_operations: 64,
    };

    pub const M7B_PILOT: Self = Self {
        max_requests: 4,
        max_query_bytes: 32 * 1024,
        max_operations: 1,
    };

    fn within(self, maximum: Self) -> bool {
        self.max_requests <= maximum.max_requests
            && self.max_query_bytes <= maximum.max_query_bytes
            && self.max_operations <= maximum.max_operations
    }

    fn min(self, other: Self) -> Self {
        Self {
            max_requests: self.max_requests.min(other.max_requests),
            max_query_bytes: self.max_query_bytes.min(other.max_query_bytes),
            max_operations: self.max_operations.min(other.max_operations),
        }
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

/// What the user allowed one plugin: its capabilities, the typed operations
/// (by their wire name, e.g. `set_dimension`) it may send, and its budgets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginGrant {
    principal_id: u64,
    capabilities: BTreeSet<PluginCapability>,
    allowed_operations: BTreeSet<String>,
    limits: PluginLimits,
}

impl PluginGrant {
    #[must_use]
    pub fn new(
        principal_id: u64,
        capabilities: impl IntoIterator<Item = PluginCapability>,
        allowed_operations: impl IntoIterator<Item = impl Into<String>>,
        limits: PluginLimits,
    ) -> Self {
        Self {
            principal_id,
            capabilities: capabilities.into_iter().collect(),
            allowed_operations: allowed_operations.into_iter().map(Into::into).collect(),
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
    pub const fn allowed_operations(&self) -> &BTreeSet<String> {
        &self.allowed_operations
    }

    #[must_use]
    pub const fn limits(&self) -> PluginLimits {
        self.limits
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginRequest {
    QueryAgentState,
    Apply(AssistantCadEditProgram),
}

impl PluginRequest {
    const fn required_capability(&self) -> PluginCapability {
        match self {
            Self::QueryAgentState => PluginCapability::QueryAgentState,
            Self::Apply(_) => PluginCapability::EditCadProgram,
        }
    }
}

/// A program that passed the gateway, bound to the revision the plugin read
/// on its first request. The caller commits it only while the document is
/// still at that revision, as MCP does with its `expected` stamp.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginProgram {
    pub revision_id: u64,
    pub program: AssistantCadEditProgram,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginResponse {
    AgentState(String),
    Program(PluginProgram),
}

pub struct PluginGateway {
    manifest: PluginManifest,
    grant: PluginGrant,
    request_count: usize,
    query_bytes: usize,
    read_revision: Option<u64>,
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
            read_revision: None,
        })
    }

    #[must_use]
    pub const fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    pub fn handle(
        &mut self,
        snapshot: &Snapshot,
        request: PluginRequest,
    ) -> Result<PluginResponse, PluginGatewayError> {
        let limits = self.manifest.limits.min(self.grant.limits);
        self.request_count = self.request_count.saturating_add(1);
        if self.request_count > limits.max_requests {
            return Err(PluginGatewayError::RequestBudgetExceeded);
        }

        let required = request.required_capability();
        if !self.manifest.capabilities.contains(&required)
            || !self.grant.capabilities.contains(&required)
        {
            return Err(PluginGatewayError::CapabilityDenied(required));
        }
        let revision_id = *self.read_revision.get_or_insert(snapshot.revision_id());

        match request {
            PluginRequest::QueryAgentState => {
                let state = encode_semantic_state(snapshot).agent();
                let next_bytes = self.query_bytes.saturating_add(state.len());
                if next_bytes > limits.max_query_bytes {
                    return Err(PluginGatewayError::QueryBudgetExceeded {
                        attempted_bytes: next_bytes,
                        max_bytes: limits.max_query_bytes,
                    });
                }
                self.query_bytes = next_bytes;
                Ok(PluginResponse::AgentState(state))
            }
            PluginRequest::Apply(program) => {
                if program.operations.len() > limits.max_operations {
                    return Err(PluginGatewayError::OperationBudgetExceeded {
                        operations: program.operations.len(),
                        max_operations: limits.max_operations,
                    });
                }
                for operation in &program.operations {
                    let name = wire_name(operation);
                    if !self.grant.allowed_operations.contains(&name) {
                        return Err(PluginGatewayError::OperationNotAllowed(name));
                    }
                }
                Ok(PluginResponse::Program(PluginProgram {
                    revision_id,
                    program,
                }))
            }
        }
    }
}

/// The `operation` tag the operation has on the wire, as MCP and plugins write it.
fn wire_name(operation: &AssistantCadEditOperation) -> String {
    serde_json::to_value(operation)
        .ok()
        .and_then(|value| value.get("operation")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[derive(Debug, PartialEq, Eq)]
pub enum PluginGatewayError {
    InvalidManifest(String),
    PrincipalMismatch,
    HostLimitExceeded,
    CapabilityDenied(PluginCapability),
    OperationNotAllowed(String),
    RequestBudgetExceeded,
    QueryBudgetExceeded {
        attempted_bytes: usize,
        max_bytes: usize,
    },
    OperationBudgetExceeded {
        operations: usize,
        max_operations: usize,
    },
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
            Self::OperationNotAllowed(name) => {
                write!(formatter, "plugin operation {name:?} was not granted")
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
            Self::OperationBudgetExceeded {
                operations,
                max_operations,
            } => write!(
                formatter,
                "plugin program has {operations} operations, {max_operations} allowed"
            ),
        }
    }
}

impl std::error::Error for PluginGatewayError {}
