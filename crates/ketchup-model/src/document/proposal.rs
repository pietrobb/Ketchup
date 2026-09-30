use super::*;

/// An immutable immediate-parent snapshot guarded by the exact tip it may replace.
#[derive(Clone)]
pub struct TipReplacementParent {
    pub(super) snapshot: Snapshot,
    pub(super) document_id: DocumentId,
    pub(super) parent_revision: u64,
    pub(super) parent_digest: String,
    pub(super) superseded_revision: u64,
    pub(super) superseded_digest: String,
}

impl TipReplacementParent {
    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn parent_revision(&self) -> u64 {
        self.parent_revision
    }

    #[must_use]
    pub fn parent_digest(&self) -> &str {
        &self.parent_digest
    }

    #[must_use]
    pub const fn superseded_revision(&self) -> u64 {
        self.superseded_revision
    }

    #[must_use]
    pub fn superseded_digest(&self) -> &str {
        &self.superseded_digest
    }
}

/// A canonical correction planned against the parent of the current tip.
#[derive(Clone)]
pub struct TipReplacementProposal {
    pub(super) document_id: DocumentId,
    pub(super) parent_revision: u64,
    pub(super) parent_digest: String,
    pub(super) superseded_revision: u64,
    pub(super) superseded_digest: String,
    pub(super) corrected_revision: u64,
    pub(super) batch: CommandBatch,
    pub(super) command_digest: String,
    pub(super) authoritative_dependencies: BTreeSet<AuthoritativeDependency>,
    pub(super) authoritative_writes: BTreeSet<AuthoritativeDependency>,
    pub(super) dependency_digest: String,
    pub(super) authoritative_diff: Vec<ProposalDiffEntry>,
    pub(super) intended_result_digest: String,
    pub(super) principal: ProposalPrincipal,
    pub(super) goal: ProposalGoal,
    pub(super) assumptions: Vec<ProposalAssumption>,
    pub(super) risk: ProposalRisk,
    pub(super) confirmation: ProposalConfirmation,
    pub(super) requested_budget: ProposalBudget,
    pub(super) cost: ProposalCost,
}

impl TipReplacementProposal {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn parent_revision(&self) -> u64 {
        self.parent_revision
    }

    #[must_use]
    pub fn parent_digest(&self) -> &str {
        &self.parent_digest
    }

    #[must_use]
    pub const fn superseded_revision(&self) -> u64 {
        self.superseded_revision
    }

    #[must_use]
    pub fn superseded_digest(&self) -> &str {
        &self.superseded_digest
    }

    #[must_use]
    pub const fn corrected_revision(&self) -> u64 {
        self.corrected_revision
    }

    #[must_use]
    pub const fn batch(&self) -> &CommandBatch {
        &self.batch
    }

    #[must_use]
    pub fn command_digest(&self) -> &str {
        &self.command_digest
    }

    #[must_use]
    pub fn dependency_digest(&self) -> &str {
        &self.dependency_digest
    }

    #[must_use]
    pub const fn authoritative_dependencies(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_dependencies
    }

    #[must_use]
    pub const fn authoritative_writes(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_writes
    }

    #[must_use]
    pub fn authoritative_diff(&self) -> &[ProposalDiffEntry] {
        &self.authoritative_diff
    }

    #[must_use]
    pub fn intended_result_digest(&self) -> &str {
        &self.intended_result_digest
    }

    #[must_use]
    pub const fn principal(&self) -> ProposalPrincipal {
        self.principal
    }

    #[must_use]
    pub fn goal(&self) -> ProposalGoal {
        self.goal.clone()
    }

    #[must_use]
    pub fn assumptions(&self) -> &[ProposalAssumption] {
        &self.assumptions
    }

    #[must_use]
    pub const fn risk(&self) -> ProposalRisk {
        self.risk
    }

    #[must_use]
    pub const fn confirmation(&self) -> &ProposalConfirmation {
        &self.confirmation
    }

    #[must_use]
    pub const fn requested_budget(&self) -> ProposalBudget {
        self.requested_budget
    }

    #[must_use]
    pub const fn cost(&self) -> ProposalCost {
        self.cost
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub(super) document_id: DocumentId,
    pub(super) provenance_revision: u64,
    pub(super) provenance_digest: String,
    pub(super) batch: CommandBatch,
    pub(super) command_digest: String,
    pub(super) authoritative_dependencies: BTreeSet<AuthoritativeDependency>,
    pub(super) authoritative_writes: BTreeSet<AuthoritativeDependency>,
    pub(super) dependency_digest: String,
    pub(super) authoritative_diff: Vec<ProposalDiffEntry>,
    pub(super) intended_result_digest: String,
    pub(super) principal: ProposalPrincipal,
    pub(super) goal: ProposalGoal,
    pub(super) assumptions: Vec<ProposalAssumption>,
    pub(super) risk: ProposalRisk,
    pub(super) confirmation: ProposalConfirmation,
    pub(super) requested_budget: ProposalBudget,
    pub(super) cost: ProposalCost,
}

impl Proposal {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn provenance_revision(&self) -> u64 {
        self.provenance_revision
    }

    #[must_use]
    pub fn provenance_digest(&self) -> &str {
        &self.provenance_digest
    }

    #[must_use]
    pub const fn batch(&self) -> &CommandBatch {
        &self.batch
    }

    #[must_use]
    pub fn command_digest(&self) -> &str {
        &self.command_digest
    }

    #[must_use]
    pub const fn authoritative_dependencies(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_dependencies
    }

    #[must_use]
    pub const fn authoritative_writes(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_writes
    }

    #[must_use]
    pub fn authoritative_diff(&self) -> &[ProposalDiffEntry] {
        &self.authoritative_diff
    }

    #[must_use]
    pub fn intended_result_digest(&self) -> &str {
        &self.intended_result_digest
    }

    #[must_use]
    pub const fn principal(&self) -> ProposalPrincipal {
        self.principal
    }

    #[must_use]
    pub fn goal(&self) -> ProposalGoal {
        self.goal.clone()
    }

    #[must_use]
    pub fn assumptions(&self) -> &[ProposalAssumption] {
        &self.assumptions
    }

    #[must_use]
    pub const fn risk(&self) -> ProposalRisk {
        self.risk
    }

    #[must_use]
    pub const fn confirmation(&self) -> &ProposalConfirmation {
        &self.confirmation
    }

    #[must_use]
    pub const fn requested_budget(&self) -> ProposalBudget {
        self.requested_budget
    }

    #[must_use]
    pub const fn cost(&self) -> ProposalCost {
        self.cost
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideEffectProposal {
    pub(super) document_id: DocumentId,
    pub(super) provenance_revision: u64,
    pub(super) provenance_digest: String,
    pub(super) operation: String,
    pub(super) operation_digest: String,
    pub(super) payload_digest: String,
    pub(super) principal: ProposalPrincipal,
    pub(super) scope: HighRiskScope,
}

impl SideEffectProposal {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn provenance_revision(&self) -> u64 {
        self.provenance_revision
    }

    #[must_use]
    pub fn provenance_digest(&self) -> &str {
        &self.provenance_digest
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    #[must_use]
    pub fn operation_digest(&self) -> &str {
        &self.operation_digest
    }

    #[must_use]
    pub fn payload_digest(&self) -> &str {
        &self.payload_digest
    }

    #[must_use]
    pub const fn principal(&self) -> ProposalPrincipal {
        self.principal
    }

    #[must_use]
    pub const fn scope(&self) -> &HighRiskScope {
        &self.scope
    }
}

pub const MAX_HUMAN_CONFIRMATION_LIFETIME_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthenticatedApprover {
    Human(u64),
    Machine(ProposalPrincipal),
}

pub struct TrustedConfirmationSurface {
    pub(super) signing_key: SigningKey,
    pub(super) policy_epoch: u64,
}

impl TrustedConfirmationSurface {
    pub fn new(signing_key: [u8; 32], policy_epoch: u64) -> Result<Self, HumanConfirmationError> {
        if policy_epoch == 0 {
            return Err(HumanConfirmationError::PolicyEpochInvalid);
        }
        Ok(Self {
            signing_key: SigningKey::from_bytes(&signing_key),
            policy_epoch,
        })
    }

    #[must_use]
    pub fn verifying_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn issue(
        &self,
        proposal: &Proposal,
        approver: AuthenticatedApprover,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<HumanApprovalToken, HumanConfirmationError> {
        let approving_human = match approver {
            AuthenticatedApprover::Human(id) if id != 0 => id,
            AuthenticatedApprover::Human(_) => {
                return Err(HumanConfirmationError::InvalidHumanPrincipal);
            }
            AuthenticatedApprover::Machine(_) => {
                return Err(HumanConfirmationError::MachineCannotApprove);
            }
        };
        if proposal.principal == ProposalPrincipal::Human(approving_human) {
            return Err(HumanConfirmationError::RequesterCannotApprove);
        }
        let ProposalRisk::High(risk_class) = proposal.risk else {
            return Err(HumanConfirmationError::NotHighRisk);
        };
        let ProposalConfirmation::HumanOnly(scope) = &proposal.confirmation else {
            return Err(HumanConfirmationError::ConfirmationRequirementMismatch);
        };
        if scope.class != risk_class {
            return Err(HumanConfirmationError::ConfirmationRequirementMismatch);
        }
        if expires_at_ms <= issued_at_ms
            || expires_at_ms - issued_at_ms > MAX_HUMAN_CONFIRMATION_LIFETIME_MS
        {
            return Err(HumanConfirmationError::InvalidLifetime);
        }
        let mut token = HumanApprovalToken {
            requester: proposal.principal,
            approving_human,
            document_id: proposal.document_id,
            revision_id: proposal.provenance_revision,
            provenance_digest: proposal.provenance_digest.clone(),
            dependency_digest: proposal.dependency_digest.clone(),
            command_digest: proposal.command_digest.clone(),
            result_digest: proposal.intended_result_digest.clone(),
            scope: scope.clone(),
            policy_epoch: self.policy_epoch,
            issued_at_ms,
            expires_at_ms,
            signature: [0; 64],
        };
        token.signature = self.signing_key.sign(&token.signing_payload()).to_bytes();
        Ok(token)
    }

    pub fn issue_side_effect(
        &self,
        proposal: &SideEffectProposal,
        approver: AuthenticatedApprover,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<SideEffectApprovalToken, HumanConfirmationError> {
        let approving_human = authenticated_human(approver)?;
        if proposal.principal == ProposalPrincipal::Human(approving_human) {
            return Err(HumanConfirmationError::RequesterCannotApprove);
        }
        validate_confirmation_lifetime(issued_at_ms, expires_at_ms)?;
        let mut token = SideEffectApprovalToken {
            requester: proposal.principal,
            approving_human,
            document_id: proposal.document_id,
            revision_id: proposal.provenance_revision,
            provenance_digest: proposal.provenance_digest.clone(),
            operation_digest: proposal.operation_digest.clone(),
            payload_digest: proposal.payload_digest.clone(),
            scope: proposal.scope.clone(),
            policy_epoch: self.policy_epoch,
            issued_at_ms,
            expires_at_ms,
            signature: [0; 64],
        };
        token.signature = self.signing_key.sign(&token.signing_payload()).to_bytes();
        Ok(token)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanApprovalToken {
    pub(super) requester: ProposalPrincipal,
    pub(super) approving_human: u64,
    pub(super) document_id: DocumentId,
    pub(super) revision_id: u64,
    pub(super) provenance_digest: String,
    pub(super) dependency_digest: String,
    pub(super) command_digest: String,
    pub(super) result_digest: String,
    pub(super) scope: HighRiskScope,
    pub(super) policy_epoch: u64,
    pub(super) issued_at_ms: u64,
    pub(super) expires_at_ms: u64,
    pub(super) signature: [u8; 64],
}

impl HumanApprovalToken {
    #[must_use]
    pub const fn approving_human(&self) -> u64 {
        self.approving_human
    }

    #[must_use]
    pub const fn policy_epoch(&self) -> u64 {
        self.policy_epoch
    }

    #[must_use]
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    pub(super) fn signing_payload(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_confirmation_field(&mut bytes, b"ketchup.human-confirmation.v1");
        push_confirmation_principal(&mut bytes, self.requester);
        push_confirmation_u64(&mut bytes, self.approving_human);
        push_confirmation_u64(&mut bytes, self.document_id.0);
        push_confirmation_u64(&mut bytes, self.revision_id);
        push_confirmation_field(&mut bytes, self.provenance_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.dependency_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.command_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.result_digest.as_bytes());
        push_confirmation_scope(&mut bytes, &self.scope);
        push_confirmation_u64(&mut bytes, self.policy_epoch);
        push_confirmation_u64(&mut bytes, self.issued_at_ms);
        push_confirmation_u64(&mut bytes, self.expires_at_ms);
        bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideEffectApprovalToken {
    pub(super) requester: ProposalPrincipal,
    pub(super) approving_human: u64,
    pub(super) document_id: DocumentId,
    pub(super) revision_id: u64,
    pub(super) provenance_digest: String,
    pub(super) operation_digest: String,
    pub(super) payload_digest: String,
    pub(super) scope: HighRiskScope,
    pub(super) policy_epoch: u64,
    pub(super) issued_at_ms: u64,
    pub(super) expires_at_ms: u64,
    pub(super) signature: [u8; 64],
}

impl SideEffectApprovalToken {
    pub(super) fn signing_payload(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_confirmation_field(&mut bytes, b"ketchup.side-effect-confirmation.v1");
        push_confirmation_principal(&mut bytes, self.requester);
        push_confirmation_u64(&mut bytes, self.approving_human);
        push_confirmation_u64(&mut bytes, self.document_id.0);
        push_confirmation_u64(&mut bytes, self.revision_id);
        push_confirmation_field(&mut bytes, self.provenance_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.operation_digest.as_bytes());
        push_confirmation_field(&mut bytes, self.payload_digest.as_bytes());
        push_confirmation_scope(&mut bytes, &self.scope);
        push_confirmation_u64(&mut bytes, self.policy_epoch);
        push_confirmation_u64(&mut bytes, self.issued_at_ms);
        push_confirmation_u64(&mut bytes, self.expires_at_ms);
        bytes
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SideEffectAuthorizationReceipt {
    pub(super) approving_human: u64,
    pub(super) document_id: DocumentId,
    pub(super) revision_id: u64,
    pub(super) operation: String,
    pub(super) operation_digest: String,
    pub(super) payload_digest: String,
    pub(super) scope: HighRiskScope,
    pub(super) policy_epoch: u64,
    pub(super) authorized_at_ms: u64,
}

impl SideEffectAuthorizationReceipt {
    #[must_use]
    pub const fn approving_human(&self) -> u64 {
        self.approving_human
    }

    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn revision_id(&self) -> u64 {
        self.revision_id
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    #[must_use]
    pub fn operation_digest(&self) -> &str {
        &self.operation_digest
    }

    #[must_use]
    pub fn payload_digest(&self) -> &str {
        &self.payload_digest
    }

    #[must_use]
    pub const fn scope(&self) -> &HighRiskScope {
        &self.scope
    }

    #[must_use]
    pub const fn policy_epoch(&self) -> u64 {
        self.policy_epoch
    }

    #[must_use]
    pub const fn authorized_at_ms(&self) -> u64 {
        self.authorized_at_ms
    }
}

#[derive(Clone)]
pub struct VerifiedProposalCommit {
    pub(super) revision: Arc<Revision>,
    pub(super) command_digest: String,
    pub(super) result_digest: String,
    pub(super) verified_writes: BTreeSet<AuthoritativeDependency>,
}

impl VerifiedProposalCommit {
    #[must_use]
    pub const fn revision(&self) -> &Arc<Revision> {
        &self.revision
    }

    #[must_use]
    pub fn command_digest(&self) -> &str {
        &self.command_digest
    }

    #[must_use]
    pub fn result_digest(&self) -> &str {
        &self.result_digest
    }

    #[must_use]
    pub const fn verified_writes(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.verified_writes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalValidity {
    Valid {
        evaluated_revision: u64,
    },
    Stale {
        provenance_revision: u64,
        current_revision: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HumanConfirmationError {
    InvalidScope,
    PolicyEpochInvalid,
    InvalidVerifyingKey,
    InvalidHumanPrincipal,
    UnidentifiedRequester,
    MachineCannotApprove,
    RequesterCannotApprove,
    NotHighRisk,
    ConfirmationRequirementMismatch,
    InvalidLifetime,
    InvalidSideEffectEvidence,
}

impl fmt::Display for HumanConfirmationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope => formatter.write_str("high-risk confirmation scope is invalid"),
            Self::PolicyEpochInvalid => {
                formatter.write_str("human-confirmation policy epoch must advance from non-zero")
            }
            Self::InvalidVerifyingKey => {
                formatter.write_str("human-confirmation verifying key is invalid")
            }
            Self::InvalidHumanPrincipal => {
                formatter.write_str("approving human principal must be authenticated and non-zero")
            }
            Self::UnidentifiedRequester => formatter.write_str(
                "high-risk proposal requires an explicitly identified requesting principal",
            ),
            Self::MachineCannotApprove => {
                formatter.write_str("machine principals cannot approve human-only operations")
            }
            Self::RequesterCannotApprove => {
                formatter.write_str("requesting human cannot satisfy distinct-human approval")
            }
            Self::NotHighRisk => {
                formatter.write_str("human-only approval applies only to high-risk proposals")
            }
            Self::ConfirmationRequirementMismatch => {
                formatter.write_str("proposal risk and confirmation requirement do not match")
            }
            Self::InvalidLifetime => formatter
                .write_str("human confirmation lifetime must be positive and at most five minutes"),
            Self::InvalidSideEffectEvidence => formatter.write_str(
                "side-effect operation and payload evidence must be non-empty and bounded",
            ),
        }
    }
}

impl std::error::Error for HumanConfirmationError {}

#[derive(Debug, PartialEq)]
pub enum ProposalPrepareError {
    HostBudgetExceeded,
    RequestedBudgetExceeded,
    Confirmation(HumanConfirmationError),
    Canonical(CanonicalError),
}

impl fmt::Display for ProposalPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostBudgetExceeded => formatter.write_str("proposal budget exceeds host policy"),
            Self::RequestedBudgetExceeded => {
                formatter.write_str("proposal work exceeds its requested budget")
            }
            Self::Confirmation(error) => error.fmt(formatter),
            Self::Canonical(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProposalPrepareError {}

impl From<CanonicalError> for ProposalPrepareError {
    fn from(error: CanonicalError) -> Self {
        Self::Canonical(error)
    }
}

#[derive(Debug, PartialEq)]
pub enum TipReplacementProposalError {
    NoCurrentTip,
    RedoBranch,
    Stale,
    Preparation(ProposalPrepareError),
    Canonical(CanonicalError),
    VerificationMismatch,
    HumanApprovalRequired,
}

impl fmt::Display for TipReplacementProposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCurrentTip => {
                formatter.write_str("tip replacement requires an immediate parent revision")
            }
            Self::RedoBranch => {
                formatter.write_str("tip replacement requires the current last revision")
            }
            Self::Stale => formatter.write_str("tip replacement provenance is stale"),
            Self::Preparation(error) => error.fmt(formatter),
            Self::Canonical(error) => error.fmt(formatter),
            Self::VerificationMismatch => {
                formatter.write_str("tip replacement proposal verification failed")
            }
            Self::HumanApprovalRequired => formatter
                .write_str("high-risk tip replacement requires authenticated human approval"),
        }
    }
}

impl std::error::Error for TipReplacementProposalError {}

#[derive(Debug, PartialEq)]
pub enum ProposalCommitError {
    Stale(ProposalValidity),
    Preparation(ProposalPrepareError),
    Canonical(CanonicalError),
    VerificationMismatch,
    HumanApprovalRequired,
    HumanApprovalUnexpected,
    HumanApprovalPolicyUnavailable,
    HumanApprovalPolicyStale,
    HumanApprovalInvalid,
    HumanApprovalReplayed,
}

impl fmt::Display for ProposalCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale(_) => formatter.write_str("proposal dependencies changed"),
            Self::Preparation(error) => error.fmt(formatter),
            Self::Canonical(error) => error.fmt(formatter),
            Self::VerificationMismatch => formatter.write_str("proposal verification failed"),
            Self::HumanApprovalRequired => {
                formatter.write_str("high-risk proposal requires authenticated human approval")
            }
            Self::HumanApprovalUnexpected => {
                formatter.write_str("human-only approval was supplied for a standard proposal")
            }
            Self::HumanApprovalPolicyUnavailable => {
                formatter.write_str("trusted human-confirmation policy is unavailable")
            }
            Self::HumanApprovalPolicyStale => {
                formatter.write_str("human-confirmation policy epoch is stale")
            }
            Self::HumanApprovalInvalid => {
                formatter.write_str("human confirmation is invalid, expired, or mismatched")
            }
            Self::HumanApprovalReplayed => {
                formatter.write_str("human confirmation was already consumed")
            }
        }
    }
}

impl std::error::Error for ProposalCommitError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideEffectAuthorizationError {
    PolicyUnavailable,
    PolicyStale,
    Invalid,
    Replayed,
}

impl fmt::Display for SideEffectAuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PolicyUnavailable => {
                formatter.write_str("trusted side-effect confirmation policy is unavailable")
            }
            Self::PolicyStale => formatter.write_str("side-effect confirmation policy is stale"),
            Self::Invalid => formatter
                .write_str("side-effect confirmation is invalid, expired, stale, or mismatched"),
            Self::Replayed => formatter.write_str("side-effect confirmation was already consumed"),
        }
    }
}

impl std::error::Error for SideEffectAuthorizationError {}

pub(super) fn validate_high_risk_requester(
    principal: ProposalPrincipal,
) -> Result<(), HumanConfirmationError> {
    if matches!(
        principal,
        ProposalPrincipal::ManualClient
            | ProposalPrincipal::Human(0)
            | ProposalPrincipal::Plugin(0)
    ) {
        return Err(HumanConfirmationError::UnidentifiedRequester);
    }
    Ok(())
}

pub(super) fn authenticated_human(
    approver: AuthenticatedApprover,
) -> Result<u64, HumanConfirmationError> {
    match approver {
        AuthenticatedApprover::Human(id) if id != 0 => Ok(id),
        AuthenticatedApprover::Human(_) => Err(HumanConfirmationError::InvalidHumanPrincipal),
        AuthenticatedApprover::Machine(_) => Err(HumanConfirmationError::MachineCannotApprove),
    }
}

pub(super) fn validate_confirmation_lifetime(
    issued_at_ms: u64,
    expires_at_ms: u64,
) -> Result<(), HumanConfirmationError> {
    if expires_at_ms <= issued_at_ms
        || expires_at_ms - issued_at_ms > MAX_HUMAN_CONFIRMATION_LIFETIME_MS
    {
        return Err(HumanConfirmationError::InvalidLifetime);
    }
    Ok(())
}

pub(super) fn validate_confirmation_requirement(
    context: &ProposalContext,
) -> Result<(), ProposalPrepareError> {
    if matches!(context.risk, ProposalRisk::High(_))
        && matches!(
            context.principal,
            ProposalPrincipal::ManualClient
                | ProposalPrincipal::Human(0)
                | ProposalPrincipal::Plugin(0)
        )
    {
        return Err(ProposalPrepareError::Confirmation(
            HumanConfirmationError::UnidentifiedRequester,
        ));
    }
    match (context.risk, &context.confirmation) {
        (ProposalRisk::Standard, ProposalConfirmation::ReviewRequired) => Ok(()),
        (ProposalRisk::High(class), ProposalConfirmation::HumanOnly(scope))
            if class == scope.class =>
        {
            Ok(())
        }
        _ => Err(ProposalPrepareError::Confirmation(
            HumanConfirmationError::ConfirmationRequirementMismatch,
        )),
    }
}

pub(super) fn push_confirmation_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

pub(super) fn push_confirmation_field(bytes: &mut Vec<u8>, value: &[u8]) {
    push_confirmation_u64(bytes, u64::try_from(value.len()).unwrap_or(u64::MAX));
    bytes.extend_from_slice(value);
}

pub(super) fn push_confirmation_optional_field(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            bytes.push(1);
            push_confirmation_field(bytes, value.as_bytes());
        }
        None => bytes.push(0),
    }
}

pub(super) fn push_confirmation_principal(bytes: &mut Vec<u8>, principal: ProposalPrincipal) {
    match principal {
        ProposalPrincipal::ManualClient => bytes.push(0),
        ProposalPrincipal::Human(id) => {
            bytes.push(1);
            push_confirmation_u64(bytes, id);
        }
        ProposalPrincipal::LocalAssistant => bytes.push(2),
        ProposalPrincipal::Plugin(id) => {
            bytes.push(3);
            push_confirmation_u64(bytes, id);
        }
    }
}

pub(super) fn push_confirmation_scope(bytes: &mut Vec<u8>, scope: &HighRiskScope) {
    bytes.push(match scope.class {
        HighRiskClass::DestructiveBulkChange => 0,
        HighRiskClass::Overwrite => 1,
        HighRiskClass::LossyConversion => 2,
        HighRiskClass::ExternalDisclosure => 3,
        HighRiskClass::ReleaseManufacturingExportWithWarnings => 4,
        HighRiskClass::CapabilityExpansion => 5,
    });
    push_confirmation_optional_field(bytes, scope.destination());
    push_confirmation_optional_field(bytes, scope.provider());
    push_confirmation_optional_field(bytes, scope.path());
}

pub(super) fn validate_proposal_budget(
    requested: ProposalBudget,
    cost: ProposalCost,
) -> Result<(), ProposalPrepareError> {
    if requested.max_commands > ProposalBudget::HOST_MAX.max_commands
        || requested.max_read_dependencies > ProposalBudget::HOST_MAX.max_read_dependencies
        || requested.max_write_targets > ProposalBudget::HOST_MAX.max_write_targets
    {
        return Err(ProposalPrepareError::HostBudgetExceeded);
    }
    if cost.commands > requested.max_commands
        || cost.read_dependencies > requested.max_read_dependencies
        || cost.write_targets > requested.max_write_targets
    {
        return Err(ProposalPrepareError::RequestedBudgetExceeded);
    }
    Ok(())
}
