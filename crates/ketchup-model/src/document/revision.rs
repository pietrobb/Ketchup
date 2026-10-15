use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionOrigin {
    Initial,
    Principal(ProposalPrincipal),
    Rollback {
        principal: ProposalPrincipal,
        target_revision: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionCatalogEntry {
    pub revision_id: u64,
    pub canonical_digest: String,
    pub batch_digest: String,
    pub origin: RevisionOrigin,
    pub checkpoint: Option<String>,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionDiffEntry {
    pub structure: &'static str,
    pub before_count: usize,
    pub after_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionDiff {
    pub before_revision: u64,
    pub after_revision: u64,
    pub before_digest: String,
    pub after_digest: String,
    pub changes: Vec<RevisionDiffEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevisionHistoryError {
    Stale,
    RevisionNotFound(u64),
    InvalidCheckpointName,
    DuplicateCheckpointName,
    InvalidPrincipal,
    NoOpRollback,
    RevisionExhausted,
}

impl fmt::Display for RevisionHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => formatter.write_str("revision history request is stale"),
            Self::RevisionNotFound(id) => write!(formatter, "revision {id} was not found"),
            Self::InvalidCheckpointName => formatter.write_str("checkpoint name is invalid"),
            Self::DuplicateCheckpointName => formatter.write_str("checkpoint name already exists"),
            Self::InvalidPrincipal => formatter.write_str("rollback principal is invalid"),
            Self::NoOpRollback => formatter.write_str("rollback target is already current"),
            Self::RevisionExhausted => formatter.write_str("revision identifiers are exhausted"),
        }
    }
}

impl std::error::Error for RevisionHistoryError {}

pub(super) fn update_revision_principal_digest(digest: &mut Sha256, principal: ProposalPrincipal) {
    match principal {
        ProposalPrincipal::ManualClient => digest.update([0]),
        ProposalPrincipal::Human(id) => {
            digest.update([1]);
            digest.update(id.to_le_bytes());
        }
        ProposalPrincipal::LocalAssistant => digest.update([2]),
        ProposalPrincipal::Plugin(id) => {
            digest.update([3]);
            digest.update(id.to_le_bytes());
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RuleProgramSource {
    pub file_name: String,
    pub source: String,
    pub overrides: BTreeMap<String, f64>,
    /// Named variants of the program: each holds the value of every parameter.
    /// Files written before variants have none; an empty map is not encoded, so
    /// a program without variants encodes and digests exactly as before.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "variant_values"
    )]
    pub variants: BTreeMap<String, BTreeMap<String, f64>>,
}

pub const MAX_RULE_PROGRAM_BYTES: usize = 1024 * 1024;
pub const MAX_RULE_PROGRAM_VARIANTS: usize = 64;

impl RuleProgramSource {
    /// Checks the content a revision may hold; the encoded size is checked by
    /// whoever encodes it.
    pub fn validate(&self) -> Result<(), RuleProgramError> {
        if self.source.is_empty() {
            return Err(RuleProgramError::EmptySource);
        }
        if !self.overrides.values().all(|value| value.is_finite()) {
            return Err(RuleProgramError::NonFiniteOverride);
        }
        if self.variants.len() > MAX_RULE_PROGRAM_VARIANTS {
            return Err(RuleProgramError::TooManyVariants {
                count: self.variants.len(),
            });
        }
        for (name, values) in &self.variants {
            if name.trim().is_empty() {
                return Err(RuleProgramError::EmptyVariantName);
            }
            if let Some((parameter, _)) = values.iter().find(|(_, value)| !value.is_finite()) {
                return Err(RuleProgramError::NonFiniteVariantValue {
                    variant: name.clone(),
                    parameter: parameter.clone(),
                });
            }
        }
        Ok(())
    }
}

/// JSON has no NaN or infinity and serde_json writes them as `null`; reading
/// that back as NaN lets [`RuleProgramSource::validate`] name the variant.
fn variant_values<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, BTreeMap<String, f64>>, D::Error> {
    let variants: BTreeMap<String, BTreeMap<String, Option<f64>>> =
        serde::Deserialize::deserialize(deserializer)?;
    Ok(variants
        .into_iter()
        .map(|(name, values)| {
            let values = values
                .into_iter()
                .map(|(parameter, value)| (parameter, value.unwrap_or(f64::NAN)))
                .collect();
            (name, values)
        })
        .collect())
}

#[derive(Clone)]
pub struct Revision {
    pub(super) id: u64,
    pub(super) snapshot: Snapshot,
    pub(super) batch_digest: String,
    pub(super) origin: RevisionOrigin,
    pub(super) checkpoint: Option<String>,
    /// Shared between the revisions that keep the same program, so an Undo history
    /// holds each program once.
    pub(super) rule_program: Option<std::sync::Arc<RuleProgramSource>>,
    pub(super) recomputed_nodes: BTreeSet<NodeId>,
    pub(super) dirty_features: BTreeSet<FeatureId>,
    pub(super) feature_states: BTreeMap<FeatureId, FeatureEvaluationState>,
    pub(super) evaluation: Option<EvaluationReport>,
}

impl Revision {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn batch_digest(&self) -> &str {
        &self.batch_digest
    }

    #[must_use]
    pub const fn origin(&self) -> RevisionOrigin {
        self.origin
    }

    #[must_use]
    pub fn checkpoint(&self) -> Option<&str> {
        self.checkpoint.as_deref()
    }

    #[must_use]
    pub fn rule_program(&self) -> Option<&RuleProgramSource> {
        self.rule_program.as_deref()
    }

    #[must_use]
    pub const fn recomputed_nodes(&self) -> &BTreeSet<NodeId> {
        &self.recomputed_nodes
    }

    #[must_use]
    pub const fn dirty_features(&self) -> &BTreeSet<FeatureId> {
        &self.dirty_features
    }

    #[must_use]
    pub const fn feature_states(&self) -> &BTreeMap<FeatureId, FeatureEvaluationState> {
        &self.feature_states
    }

    #[must_use]
    pub const fn evaluation(&self) -> Option<&EvaluationReport> {
        self.evaluation.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct DerivedResultKey {
    pub document_id: DocumentId,
    pub revision_id: u64,
    pub root_rule_node_id: NodeId,
    pub slot_path: SlotPath,
    pub input_digest: String,
    pub result_digest: String,
    pub evaluator: String,
    pub backend: Option<String>,
    pub schema: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DerivedResultClassification {
    Current,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactReferenceRebind {
    pub lineage_digest: String,
    pub resolution: ExactReferenceResolution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DerivedResultPayload {
    Evaluation(DerivedResultKey),
    ExactReference(BodySubshapeRef),
    ExactReferenceRebinds(Vec<ExactReferenceRebind>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedResultEvent {
    pub document_id: DocumentId,
    pub revision_id: u64,
    pub canonical_digest: String,
    pub classification: DerivedResultClassification,
    pub payload: DerivedResultPayload,
}

pub enum ExactReferenceEvidence {
    Reference(Box<BodySubshapeRef>),
    Registry(ExactResultRegistry),
}

impl From<BodySubshapeRef> for ExactReferenceEvidence {
    fn from(reference: BodySubshapeRef) -> Self {
        Self::Reference(Box::new(reference))
    }
}

impl From<&ExactResultRegistry> for ExactReferenceEvidence {
    fn from(results: &ExactResultRegistry) -> Self {
        Self::Registry(results.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReferenceEvidenceError {
    InvalidLineage,
    WrongDocument,
    ProducerNotFound,
    ProducerDefinitionMismatch,
}

impl fmt::Display for ReferenceEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLineage => formatter.write_str("exact reference lineage is invalid"),
            Self::WrongDocument => {
                formatter.write_str("exact reference belongs to another document")
            }
            Self::ProducerNotFound => formatter.write_str("exact reference producer was not found"),
            Self::ProducerDefinitionMismatch => {
                formatter.write_str("exact reference producer belongs to another definition")
            }
        }
    }
}

impl std::error::Error for ReferenceEvidenceError {}

#[derive(Clone)]
pub(super) struct HumanConfirmationPolicy {
    pub(super) verifying_key: VerifyingKey,
    pub(super) epoch: u64,
    pub(super) consumed_signatures: BTreeSet<[u8; 64]>,
}

/// A snapshot a single-revision store validated when it was loaded, with the
/// feature states that store computed, so a restored history keeps them instead
/// of validating the revision a second time.
#[derive(Clone)]
pub(crate) struct ValidatedSnapshot {
    pub(super) snapshot: Snapshot,
    pub(super) feature_states: BTreeMap<FeatureId, FeatureEvaluationState>,
}

impl ValidatedSnapshot {
    pub(crate) const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

pub(crate) type StoredRevision = (
    ValidatedSnapshot,
    String,
    RevisionOrigin,
    Option<String>,
    Option<std::sync::Arc<RuleProgramSource>>,
);
