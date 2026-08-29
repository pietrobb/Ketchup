use crate::document::{DefinitionId, DocumentId, FeatureId, Snapshot};
use crate::exact_product::{BodyResultIdentity, BodySubshapeRef, ReferenceStability};
use crate::graph::sha256_hex;
use std::fmt;

pub const TOPOLOGICAL_ELEMENT_REF_SCHEMA_V1: &str = "ketchup.topological-element-ref.v1";
pub const MAX_GENERATED_TOPOLOGICAL_REFERENCES: u64 = 1_000_000;
const TOPOLOGICAL_REFERENCE_MAGIC: &[u8; 14] = b"KETCHUPTOPOREF";
const TOPOLOGICAL_REFERENCE_BINARY_V1: u16 = 1;
const MAX_TOPOLOGICAL_REFERENCE_BYTES: usize = 128 * 1024;
const MAX_TOPOLOGICAL_REFERENCE_TOKEN_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TopologicalElementKind {
    Face,
    Edge,
    Vertex,
}

impl TopologicalElementKind {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Face => "face",
            Self::Edge => "edge",
            Self::Vertex => "vertex",
        }
    }

    fn from_exact_type(value: &str) -> Option<Self> {
        match value {
            "face" | "planar_face" | "cylindrical_face" => Some(Self::Face),
            "edge" => Some(Self::Edge),
            "vertex" => Some(Self::Vertex),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TopologicalReferenceStability {
    Guaranteed,
    BestEffort,
    Ephemeral,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologicalReferenceQuarantineReason {
    InvalidLineage,
    WrongDocument,
    IncompatibleEvaluationEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TopologicalReferenceResolution {
    Resolved {
        reference: Box<TopologicalElementRef>,
    },
    Ambiguous {
        candidate_count: usize,
    },
    Lost,
    Quarantined {
        reason: TopologicalReferenceQuarantineReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TopologicalReferenceError {
    InvalidIdentity,
    InvalidBodyReference,
    InvalidEncoding,
    UnsupportedElementType(String),
    ResourceLimit,
}

impl fmt::Display for TopologicalReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity => formatter.write_str("invalid topological identity"),
            Self::InvalidBodyReference => formatter.write_str("invalid body subshape reference"),
            Self::InvalidEncoding => formatter.write_str("invalid topological reference encoding"),
            Self::UnsupportedElementType(value) => {
                write!(formatter, "unsupported topological element type {value}")
            }
            Self::ResourceLimit => formatter.write_str("topological reference limit exceeded"),
        }
    }
}

impl std::error::Error for TopologicalReferenceError {}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TopologicalElementRef {
    pub schema: String,
    pub document_id: DocumentId,
    pub definition_id: DefinitionId,
    pub source_feature_id: FeatureId,
    pub producer_feature_id: FeatureId,
    pub kind: TopologicalElementKind,
    pub source_element_id: String,
    pub producer_element_id: String,
    pub stability: TopologicalReferenceStability,
    pub lineage_digest: String,
    pub evaluator: String,
    pub backend: String,
    pub tolerance: String,
    pub result_fingerprint: String,
    pub corroborating_geometry_fingerprint: String,
}

impl TopologicalElementRef {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_id: DocumentId,
        definition_id: DefinitionId,
        source_feature_id: FeatureId,
        producer_feature_id: FeatureId,
        kind: TopologicalElementKind,
        source_element_id: impl Into<String>,
        producer_element_id: impl Into<String>,
        stability: TopologicalReferenceStability,
        evaluator: impl Into<String>,
        backend: impl Into<String>,
        tolerance: impl Into<String>,
        result_fingerprint: impl Into<String>,
        corroborating_geometry_fingerprint: impl Into<String>,
    ) -> Result<Self, TopologicalReferenceError> {
        let mut reference = Self {
            schema: TOPOLOGICAL_ELEMENT_REF_SCHEMA_V1.to_owned(),
            document_id,
            definition_id,
            source_feature_id,
            producer_feature_id,
            kind,
            source_element_id: source_element_id.into(),
            producer_element_id: producer_element_id.into(),
            stability,
            lineage_digest: String::new(),
            evaluator: evaluator.into(),
            backend: backend.into(),
            tolerance: tolerance.into(),
            result_fingerprint: result_fingerprint.into(),
            corroborating_geometry_fingerprint: corroborating_geometry_fingerprint.into(),
        };
        if !reference.has_complete_identity() {
            return Err(TopologicalReferenceError::InvalidIdentity);
        }
        reference.lineage_digest = canonical_topological_lineage_digest(&reference);
        Ok(reference)
    }

    pub fn from_body_subshape(
        reference: &BodySubshapeRef,
    ) -> Result<Self, TopologicalReferenceError> {
        if !reference.has_valid_lineage() {
            return Err(TopologicalReferenceError::InvalidBodyReference);
        }
        let kind =
            TopologicalElementKind::from_exact_type(&reference.expected_type).ok_or_else(|| {
                TopologicalReferenceError::UnsupportedElementType(reference.expected_type.clone())
            })?;
        let stability = match reference.stability {
            ReferenceStability::Guaranteed => TopologicalReferenceStability::Guaranteed,
        };
        Self::new(
            reference.document_id,
            reference.definition_id,
            reference.profile_feature_id,
            reference.producer_feature_id,
            kind,
            reference.source_element_id.clone(),
            reference.semantic_role.clone(),
            stability,
            reference.evaluator.clone(),
            reference.backend.clone(),
            reference.tolerance.clone(),
            reference.result_fingerprint.clone(),
            reference.corroborating_geometry_fingerprint.clone(),
        )
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, TopologicalReferenceError> {
        if !self.has_valid_lineage() {
            return Err(TopologicalReferenceError::InvalidIdentity);
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(TOPOLOGICAL_REFERENCE_MAGIC);
        bytes.extend_from_slice(&TOPOLOGICAL_REFERENCE_BINARY_V1.to_le_bytes());
        for value in [
            self.document_id.0,
            self.definition_id.0,
            self.source_feature_id.0,
            self.producer_feature_id.0,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(match self.kind {
            TopologicalElementKind::Face => 1,
            TopologicalElementKind::Edge => 2,
            TopologicalElementKind::Vertex => 3,
        });
        bytes.push(match self.stability {
            TopologicalReferenceStability::Guaranteed => 1,
            TopologicalReferenceStability::BestEffort => 2,
            TopologicalReferenceStability::Ephemeral => 3,
        });
        for token in [
            &self.schema,
            &self.source_element_id,
            &self.producer_element_id,
            &self.lineage_digest,
            &self.evaluator,
            &self.backend,
            &self.tolerance,
            &self.result_fingerprint,
            &self.corroborating_geometry_fingerprint,
        ] {
            push_topological_reference_token(&mut bytes, token)?;
        }
        let checksum = sha256_hex(&bytes);
        bytes.extend_from_slice(checksum.as_bytes());
        if bytes.len() > MAX_TOPOLOGICAL_REFERENCE_BYTES {
            return Err(TopologicalReferenceError::ResourceLimit);
        }
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TopologicalReferenceError> {
        if bytes.len() > MAX_TOPOLOGICAL_REFERENCE_BYTES {
            return Err(TopologicalReferenceError::ResourceLimit);
        }
        let checksum_offset = bytes
            .len()
            .checked_sub(64)
            .ok_or(TopologicalReferenceError::InvalidEncoding)?;
        let (payload, checksum) = bytes.split_at(checksum_offset);
        if sha256_hex(payload).as_bytes() != checksum {
            return Err(TopologicalReferenceError::InvalidEncoding);
        }
        let mut cursor = TopologicalReferenceCursor::new(payload);
        if cursor.take(TOPOLOGICAL_REFERENCE_MAGIC.len())? != TOPOLOGICAL_REFERENCE_MAGIC
            || cursor.read_u16()? != TOPOLOGICAL_REFERENCE_BINARY_V1
        {
            return Err(TopologicalReferenceError::InvalidEncoding);
        }
        let document_id = DocumentId(cursor.read_u64()?);
        let definition_id = DefinitionId(cursor.read_u64()?);
        let source_feature_id = FeatureId(cursor.read_u64()?);
        let producer_feature_id = FeatureId(cursor.read_u64()?);
        let kind = match cursor.read_u8()? {
            1 => TopologicalElementKind::Face,
            2 => TopologicalElementKind::Edge,
            3 => TopologicalElementKind::Vertex,
            _ => return Err(TopologicalReferenceError::InvalidEncoding),
        };
        let stability = match cursor.read_u8()? {
            1 => TopologicalReferenceStability::Guaranteed,
            2 => TopologicalReferenceStability::BestEffort,
            3 => TopologicalReferenceStability::Ephemeral,
            _ => return Err(TopologicalReferenceError::InvalidEncoding),
        };
        let reference = Self {
            schema: cursor.read_token()?,
            document_id,
            definition_id,
            source_feature_id,
            producer_feature_id,
            kind,
            source_element_id: cursor.read_token()?,
            producer_element_id: cursor.read_token()?,
            stability,
            lineage_digest: cursor.read_token()?,
            evaluator: cursor.read_token()?,
            backend: cursor.read_token()?,
            tolerance: cursor.read_token()?,
            result_fingerprint: cursor.read_token()?,
            corroborating_geometry_fingerprint: cursor.read_token()?,
        };
        if !cursor.is_finished() || !reference.has_valid_lineage() {
            return Err(TopologicalReferenceError::InvalidEncoding);
        }
        Ok(reference)
    }

    #[must_use]
    pub fn has_valid_lineage(&self) -> bool {
        self.schema == TOPOLOGICAL_ELEMENT_REF_SCHEMA_V1
            && self.has_complete_identity()
            && self.lineage_digest == canonical_topological_lineage_digest(self)
    }

    fn has_complete_identity(&self) -> bool {
        self.document_id.0 != 0
            && self.definition_id.0 != 0
            && self.source_feature_id.0 != 0
            && self.producer_feature_id.0 != 0
            && !self.source_element_id.is_empty()
            && !self.producer_element_id.is_empty()
            && !self.evaluator.is_empty()
            && !self.backend.is_empty()
            && !self.tolerance.is_empty()
            && !self.result_fingerprint.is_empty()
            && !self.corroborating_geometry_fingerprint.is_empty()
    }

    fn has_same_durable_identity(&self, other: &Self) -> bool {
        self.document_id == other.document_id
            && self.definition_id == other.definition_id
            && self.source_feature_id == other.source_feature_id
            && self.producer_feature_id == other.producer_feature_id
            && self.kind == other.kind
            && self.source_element_id == other.source_element_id
            && self.producer_element_id == other.producer_element_id
            && self.stability == other.stability
            && self.lineage_digest == other.lineage_digest
    }

    fn has_compatible_evaluation_envelope(&self, other: &Self) -> bool {
        self.evaluator == other.evaluator
            && self.backend == other.backend
            && self.tolerance == other.tolerance
    }
}

fn push_topological_reference_token(
    bytes: &mut Vec<u8>,
    token: &str,
) -> Result<(), TopologicalReferenceError> {
    if token.len() > MAX_TOPOLOGICAL_REFERENCE_TOKEN_BYTES {
        return Err(TopologicalReferenceError::ResourceLimit);
    }
    let length =
        u32::try_from(token.len()).map_err(|_| TopologicalReferenceError::ResourceLimit)?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(token.as_bytes());
    Ok(())
}

struct TopologicalReferenceCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> TopologicalReferenceCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], TopologicalReferenceError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(TopologicalReferenceError::InvalidEncoding)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(TopologicalReferenceError::InvalidEncoding)?;
        self.offset = end;
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<u8, TopologicalReferenceError> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn read_u16(&mut self) -> Result<u16, TopologicalReferenceError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| TopologicalReferenceError::InvalidEncoding)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32, TopologicalReferenceError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| TopologicalReferenceError::InvalidEncoding)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64, TopologicalReferenceError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| TopologicalReferenceError::InvalidEncoding)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn read_token(&mut self) -> Result<String, TopologicalReferenceError> {
        let length = usize::try_from(self.read_u32()?)
            .map_err(|_| TopologicalReferenceError::ResourceLimit)?;
        if length > MAX_TOPOLOGICAL_REFERENCE_TOKEN_BYTES {
            return Err(TopologicalReferenceError::ResourceLimit);
        }
        let token = std::str::from_utf8(self.take(length)?)
            .map_err(|_| TopologicalReferenceError::InvalidEncoding)?;
        Ok(token.to_owned())
    }

    const fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[must_use]
pub fn canonical_topological_lineage_digest(reference: &TopologicalElementRef) -> String {
    let mut identity = b"ketchup.topological-lineage.v1".to_vec();
    for value in [
        reference.document_id.0,
        reference.definition_id.0,
        reference.source_feature_id.0,
        reference.producer_feature_id.0,
    ] {
        identity.extend_from_slice(&value.to_le_bytes());
    }
    identity.push(match reference.kind {
        TopologicalElementKind::Face => 1,
        TopologicalElementKind::Edge => 2,
        TopologicalElementKind::Vertex => 3,
    });
    identity.push(match reference.stability {
        TopologicalReferenceStability::Guaranteed => 1,
        TopologicalReferenceStability::BestEffort => 2,
        TopologicalReferenceStability::Ephemeral => 3,
    });
    for token in [
        reference.source_element_id.as_bytes(),
        reference.producer_element_id.as_bytes(),
    ] {
        identity.extend_from_slice(&(token.len() as u64).to_le_bytes());
        identity.extend_from_slice(token);
    }
    sha256_hex(&identity)
}

pub fn publish_generated_topological_references(
    identity: &BodyResultIdentity,
    topology_counts: [u32; 5],
) -> Result<Vec<TopologicalElementRef>, TopologicalReferenceError> {
    let reference_count = topology_counts[..3]
        .iter()
        .try_fold(0_u64, |total, count| total.checked_add(u64::from(*count)))
        .ok_or(TopologicalReferenceError::ResourceLimit)?;
    if reference_count > MAX_GENERATED_TOPOLOGICAL_REFERENCES {
        return Err(TopologicalReferenceError::ResourceLimit);
    }

    let mut references = Vec::with_capacity(reference_count as usize);
    for (kind, count) in [
        (TopologicalElementKind::Vertex, topology_counts[0]),
        (TopologicalElementKind::Edge, topology_counts[1]),
        (TopologicalElementKind::Face, topology_counts[2]),
    ] {
        for ordinal in 0..count {
            let source_element_id = format!("generated-source/{}/{ordinal}", kind.token());
            let producer_element_id = format!("generated-result/{}/{ordinal}", kind.token());
            let corroborating_geometry_fingerprint =
                generated_topological_evidence_fingerprint(identity, kind, ordinal);
            references.push(TopologicalElementRef::new(
                identity.document_id,
                identity.definition_id,
                identity.producer_feature_id,
                identity.producer_feature_id,
                kind,
                source_element_id,
                producer_element_id,
                TopologicalReferenceStability::Ephemeral,
                identity.evaluator.clone(),
                identity.backend.clone(),
                identity.tolerance.clone(),
                identity.result_fingerprint.clone(),
                corroborating_geometry_fingerprint,
            )?);
        }
    }
    Ok(references)
}

pub fn publish_imported_topological_references(
    identity: &BodyResultIdentity,
    source_sha256: &[u8; 32],
    topology_counts: [u32; 5],
) -> Result<Vec<TopologicalElementRef>, TopologicalReferenceError> {
    let reference_count = topology_counts[..3]
        .iter()
        .try_fold(0_u64, |total, count| total.checked_add(u64::from(*count)))
        .ok_or(TopologicalReferenceError::ResourceLimit)?;
    if reference_count > MAX_GENERATED_TOPOLOGICAL_REFERENCES {
        return Err(TopologicalReferenceError::ResourceLimit);
    }

    let source_digest = source_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut references = Vec::with_capacity(reference_count as usize);
    for (kind, count) in [
        (TopologicalElementKind::Vertex, topology_counts[0]),
        (TopologicalElementKind::Edge, topology_counts[1]),
        (TopologicalElementKind::Face, topology_counts[2]),
    ] {
        for ordinal in 0..count {
            let source_element_id =
                format!("imported-source/{source_digest}/{}/{ordinal}", kind.token());
            let producer_element_id = format!("imported-result/{}/{ordinal}", kind.token());
            let corroborating_geometry_fingerprint =
                imported_topological_evidence_fingerprint(identity, source_sha256, kind, ordinal);
            references.push(TopologicalElementRef::new(
                identity.document_id,
                identity.definition_id,
                identity.producer_feature_id,
                identity.producer_feature_id,
                kind,
                source_element_id,
                producer_element_id,
                TopologicalReferenceStability::BestEffort,
                identity.evaluator.clone(),
                identity.backend.clone(),
                identity.tolerance.clone(),
                identity.result_fingerprint.clone(),
                corroborating_geometry_fingerprint,
            )?);
        }
    }
    Ok(references)
}

fn imported_topological_evidence_fingerprint(
    identity: &BodyResultIdentity,
    source_sha256: &[u8; 32],
    kind: TopologicalElementKind,
    ordinal: u32,
) -> String {
    let mut evidence = b"ketchup.imported-topology-evidence.v1".to_vec();
    evidence.extend_from_slice(source_sha256);
    for token in [
        identity.result_fingerprint.as_bytes(),
        identity.backend.as_bytes(),
        identity.tolerance.as_bytes(),
    ] {
        evidence.extend_from_slice(&(token.len() as u64).to_le_bytes());
        evidence.extend_from_slice(token);
    }
    evidence.push(match kind {
        TopologicalElementKind::Face => 1,
        TopologicalElementKind::Edge => 2,
        TopologicalElementKind::Vertex => 3,
    });
    evidence.extend_from_slice(&ordinal.to_le_bytes());
    sha256_hex(&evidence)
}

fn generated_topological_evidence_fingerprint(
    identity: &BodyResultIdentity,
    kind: TopologicalElementKind,
    ordinal: u32,
) -> String {
    let mut evidence = b"ketchup.generated-topology-evidence.v1".to_vec();
    for token in [
        identity.exact_input_digest.as_bytes(),
        identity.result_fingerprint.as_bytes(),
    ] {
        evidence.extend_from_slice(&(token.len() as u64).to_le_bytes());
        evidence.extend_from_slice(token);
    }
    evidence.push(match kind {
        TopologicalElementKind::Face => 1,
        TopologicalElementKind::Edge => 2,
        TopologicalElementKind::Vertex => 3,
    });
    evidence.extend_from_slice(&ordinal.to_le_bytes());
    sha256_hex(&evidence)
}

#[must_use]
pub fn resolve_topological_reference<'a>(
    snapshot: &Snapshot,
    reference: &TopologicalElementRef,
    candidates: impl IntoIterator<Item = &'a TopologicalElementRef>,
) -> TopologicalReferenceResolution {
    if !reference.has_valid_lineage() {
        return TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::InvalidLineage,
        };
    }
    if reference.document_id != snapshot.document_id() {
        return TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::WrongDocument,
        };
    }
    if snapshot
        .feature(reference.producer_feature_id)
        .is_none_or(|producer| producer.definition_id() != reference.definition_id)
    {
        return TopologicalReferenceResolution::Lost;
    }

    let matches = candidates
        .into_iter()
        .filter(|candidate| {
            candidate.has_valid_lineage() && candidate.has_same_durable_identity(reference)
        })
        .collect::<Vec<_>>();
    let [candidate] = matches.as_slice() else {
        return if matches.is_empty() {
            TopologicalReferenceResolution::Lost
        } else {
            TopologicalReferenceResolution::Ambiguous {
                candidate_count: matches.len(),
            }
        };
    };
    if !candidate.has_compatible_evaluation_envelope(reference) {
        return TopologicalReferenceResolution::Quarantined {
            reason: TopologicalReferenceQuarantineReason::IncompatibleEvaluationEnvelope,
        };
    }
    if reference.stability == TopologicalReferenceStability::Ephemeral
        && candidate.result_fingerprint != reference.result_fingerprint
    {
        return TopologicalReferenceResolution::Lost;
    }
    TopologicalReferenceResolution::Resolved {
        reference: Box::new((*candidate).clone()),
    }
}
