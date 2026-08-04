use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use crate::document::{
    CanonicalError, Definition, DefinitionId, Dimension, DocumentStore, EvaluatorNode, Feature,
    FeatureId, FeatureKind, Group, GroupId, LocalGroup, LocalGroupId, LocalGroupKey,
    LocalOccurrence, LocalOccurrenceId, LocalOccurrenceKey, NodeId, Occurrence, OccurrenceId,
    ProductModel, Snapshot, TagId, Transform, UnitSystem,
};
use crate::exact_product::{BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ReferenceStability};
use crate::graph::{
    CanonicalOverride, DerivedIdentity, EvaluatorNodeKind, OverrideMergePolicy,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment,
};
use crate::prismatic::{Aabb, CanonicalJoint, JointId};

const MAGIC: &[u8; 10] = b"KETCHUPDOC";
const CURRENT_SCHEMA: u16 = 4;
const ENVELOPE_SCHEMA: u16 = 3;
const PRODUCT_SCHEMA: u16 = 2;
const RESEARCH_SCHEMA: u16 = 1;
const LEGACY_SCHEMA: u16 = 0;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const HEADER_BYTES: usize = 16;
const MANIFEST_BYTES: usize = 8
    + 32
    + 4
    + crate::graph::GRAPH_SCHEMA_ID_V1.len()
    + 4
    + crate::graph::EVALUATOR_ID_V1.len()
    + 4
    + crate::document::TOLERANCE_PROFILE_V1.len();
const MAX_MANIFEST_BYTES: usize = MANIFEST_BYTES;
const MAX_PAYLOAD_BYTES: usize = MAX_FILE_BYTES - HEADER_BYTES - MANIFEST_BYTES;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_COLLECTION_ITEMS: u32 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationLoss {
    pub node_id: NodeId,
    pub field: &'static str,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadDisposition {
    EditableLossless,
    ReviewOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverrideHealthAudit {
    pub override_id: u64,
    pub stored: SlotResolution,
    pub audited: SlotResolution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadAudit {
    pub source_schema: u16,
    pub migration_losses: Vec<MigrationLoss>,
    pub override_health: Vec<OverrideHealthAudit>,
}

pub struct ReviewCandidate {
    snapshot: Snapshot,
    audit: LoadAudit,
}

impl ReviewCandidate {
    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn audit(&self) -> &LoadAudit {
        &self.audit
    }
}

pub enum LoadOutcome {
    Editable {
        document: DocumentStore,
        audit: LoadAudit,
    },
    ReviewOnly(ReviewCandidate),
}

impl LoadOutcome {
    #[must_use]
    pub const fn disposition(&self) -> LoadDisposition {
        match self {
            Self::Editable { .. } => LoadDisposition::EditableLossless,
            Self::ReviewOnly(_) => LoadDisposition::ReviewOnly,
        }
    }
    #[must_use]
    pub const fn is_editable(&self) -> bool {
        matches!(self, Self::Editable { .. })
    }
    #[must_use]
    pub const fn audit(&self) -> &LoadAudit {
        match self {
            Self::Editable { audit, .. } => audit,
            Self::ReviewOnly(candidate) => candidate.audit(),
        }
    }
    #[must_use]
    pub const fn source_schema(&self) -> u16 {
        self.audit().source_schema
    }
    #[must_use]
    pub fn migration_losses(&self) -> &[MigrationLoss] {
        &self.audit().migration_losses
    }
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        match self {
            Self::Editable { document, .. } => document.current(),
            Self::ReviewOnly(candidate) => candidate.snapshot.clone(),
        }
    }
    /// Read-only compatibility accessor. It never exposes the backing `DocumentStore`.
    #[must_use]
    pub fn document(&self) -> Snapshot {
        self.snapshot()
    }
    #[must_use]
    pub const fn editable_document(&self) -> Option<&DocumentStore> {
        match self {
            Self::Editable { document, .. } => Some(document),
            Self::ReviewOnly(_) => None,
        }
    }
    #[must_use]
    pub fn editable_document_mut(&mut self) -> Option<&mut DocumentStore> {
        match self {
            Self::Editable { document, .. } => Some(document),
            Self::ReviewOnly(_) => None,
        }
    }
    pub fn into_editable(self) -> Result<DocumentStore, ReviewCandidate> {
        match self {
            Self::Editable { document, .. } => Ok(document),
            Self::ReviewOnly(candidate) => Err(candidate),
        }
    }
    #[must_use]
    pub const fn review_candidate(&self) -> Option<&ReviewCandidate> {
        match self {
            Self::ReviewOnly(candidate) => Some(candidate),
            Self::Editable { .. } => None,
        }
    }
}

#[must_use]
pub fn save(snapshot: &Snapshot) -> Vec<u8> {
    let product = snapshot.product();
    let mut payload = Vec::new();
    push_u64(&mut payload, snapshot.revision_id());
    push_u64(&mut payload, product.document_id.0);
    push_u8(&mut payload, 1);
    write_nodes(&mut payload, &product.evaluator_nodes);
    push_u32(&mut payload, product.overrides.len() as u32);
    for value in product.overrides.values() {
        write_override(&mut payload, value);
    }

    push_u32(&mut payload, product.definitions.len() as u32);
    for definition in product.definitions.values() {
        push_u64(&mut payload, definition.id().0);
        push_string(&mut payload, definition.name());
        write_ids(&mut payload, definition.feature_ids().iter().map(|id| id.0));
        write_ids(
            &mut payload,
            definition.local_group_ids().iter().map(|id| id.0),
        );
        write_ids(
            &mut payload,
            definition.local_occurrence_ids().iter().map(|id| id.0),
        );
    }
    write_features(&mut payload, product);
    write_occurrences(&mut payload, product);
    write_groups(&mut payload, product);

    push_u32(&mut payload, product.local_groups.len() as u32);
    for group in product.local_groups.values() {
        push_u64(&mut payload, group.key().definition_id.0);
        push_u64(&mut payload, group.key().local_id.0);
        push_string(&mut payload, group.name());
        push_transform(&mut payload, group.transform());
        push_optional_id(&mut payload, group.parent().map(|id| id.0));
    }
    push_u32(&mut payload, product.local_occurrences.len() as u32);
    for occurrence in product.local_occurrences.values() {
        push_u64(&mut payload, occurrence.key().definition_id.0);
        push_u64(&mut payload, occurrence.key().local_id.0);
        push_u64(&mut payload, occurrence.definition_id().0);
        push_string(&mut payload, occurrence.name());
        push_transform(&mut payload, occurrence.transform());
        push_optional_id(&mut payload, occurrence.parent().map(|id| id.0));
        push_optional_id(&mut payload, occurrence.tag().map(|id| id.0));
        push_u8(&mut payload, u8::from(occurrence.visible()));
    }
    push_u32(&mut payload, product.joints.len() as u32);
    for joint in product.joints.values() {
        write_joint(&mut payload, joint);
    }
    push_u32(&mut payload, product.exact_reference_evidence.len() as u32);
    for reference in product.exact_reference_evidence.values() {
        write_exact_reference(&mut payload, reference);
    }

    let mut manifest = Vec::new();
    push_u64(&mut manifest, payload.len() as u64);
    manifest.extend_from_slice(&crate::graph::sha256_bytes(&payload));
    push_string(&mut manifest, crate::graph::GRAPH_SCHEMA_ID_V1);
    push_string(&mut manifest, crate::graph::EVALUATOR_ID_V1);
    push_string(&mut manifest, crate::document::TOLERANCE_PROFILE_V1);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    push_u16(&mut bytes, CURRENT_SCHEMA);
    push_u32(&mut bytes, manifest.len() as u32);
    bytes.extend_from_slice(&manifest);
    bytes.extend_from_slice(&payload);
    bytes
}

fn write_nodes(bytes: &mut Vec<u8>, nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>) {
    push_u32(bytes, nodes.len() as u32);
    for node in nodes.values() {
        push_u64(bytes, node.id().0);
        push_string(bytes, node.name());
        match node.kind() {
            EvaluatorNodeKind::Parameter { value } => {
                push_u8(bytes, 1);
                push_string(bytes, value.source_token());
                push_u64(bytes, value.millimetres().to_bits());
                write_ids(bytes, node.dependencies().iter().map(|id| id.0));
            }
            EvaluatorNodeKind::Expression { source, .. } => {
                push_u8(bytes, 2);
                push_string(bytes, source);
            }
            EvaluatorNodeKind::Rule {
                source, outputs, ..
            } => {
                push_u8(bytes, 3);
                push_string(bytes, source);
                write_ports(bytes, node.input_ports());
                write_ports(bytes, node.output_ports());
                write_rule_outputs(bytes, outputs);
                write_override_parameters(bytes, node.allowed_parameters());
            }
        }
    }
}

fn write_ports(bytes: &mut Vec<u8>, ports: &[PortSpec]) {
    push_u32(bytes, ports.len() as u32);
    for port in ports {
        push_string(bytes, port.name());
        push_u8(bytes, 1);
    }
}

fn write_rule_outputs(bytes: &mut Vec<u8>, outputs: &[RuleOutput]) {
    push_u32(bytes, outputs.len() as u32);
    let mut stack = outputs.iter().rev().collect::<Vec<_>>();
    while let Some(output) = stack.pop() {
        let segment = output.segment();
        push_u64(bytes, segment.producer_rule_id.0);
        push_string(bytes, &segment.output_port);
        push_string(bytes, &segment.semantic_key);
        push_u32(bytes, output.children().len() as u32);
        stack.extend(output.children().iter().rev());
    }
}

fn write_override_parameters(bytes: &mut Vec<u8>, parameters: &[OverrideParameterSpec]) {
    push_u32(bytes, parameters.len() as u32);
    for parameter in parameters {
        push_string(bytes, parameter.name());
        push_u8(
            bytes,
            match parameter.merge_policy() {
                OverrideMergePolicy::Replace => 1,
            },
        );
    }
}

fn write_slot_path(bytes: &mut Vec<u8>, path: &SlotPath) {
    push_u32(bytes, path.segments().len() as u32);
    for segment in path.segments() {
        push_u64(bytes, segment.producer_rule_id.0);
        push_string(bytes, &segment.output_port);
        push_string(bytes, &segment.semantic_key);
    }
}

fn write_identity(bytes: &mut Vec<u8>, value: &DerivedIdentity) {
    push_u64(bytes, value.root_rule_node_id.0);
    write_slot_path(bytes, &value.slot_path);
}

fn write_joint(bytes: &mut Vec<u8>, value: &CanonicalJoint) {
    push_u64(bytes, value.id().0);
    write_identity(bytes, value.participant_a());
    write_identity(bytes, value.participant_b());
    for coordinate in value.volume().min().into_iter().chain(value.volume().max()) {
        push_u64(bytes, coordinate.to_bits());
    }
}

fn write_exact_reference(bytes: &mut Vec<u8>, value: &BodySubshapeRef) {
    push_string(bytes, &value.schema);
    push_u64(bytes, value.document_id.0);
    push_u64(bytes, value.definition_id.0);
    push_u64(bytes, value.profile_feature_id.0);
    push_u64(bytes, value.producer_feature_id.0);
    push_string(bytes, &value.semantic_role);
    push_string(bytes, &value.source_element_id);
    push_string(bytes, &value.expected_type);
    push_u32(bytes, value.expected_cardinality);
    push_u8(
        bytes,
        match value.stability {
            ReferenceStability::Guaranteed => 1,
        },
    );
    push_string(bytes, &value.canonical_input_digest);
    push_string(bytes, &value.exact_input_digest);
    push_string(bytes, &value.result_fingerprint);
    push_string(bytes, &value.evaluator);
    push_string(bytes, &value.backend);
    push_string(bytes, &value.tolerance);
    push_string(bytes, &value.lineage_digest);
    push_string(bytes, &value.corroborating_geometry_fingerprint);
}

fn write_override(bytes: &mut Vec<u8>, value: &CanonicalOverride) {
    push_u64(bytes, value.id);
    push_u64(bytes, value.target.root_rule_node_id.0);
    write_slot_path(bytes, &value.target.slot_path);
    push_string(bytes, &value.parameter);
    push_u64(bytes, value.value_bits);
    match value.health {
        SlotResolution::Resolved => push_u8(bytes, 1),
        SlotResolution::Ambiguous { segment_index } => {
            push_u8(bytes, 2);
            push_u32(bytes, segment_index as u32);
        }
        SlotResolution::Lost { segment_index } => {
            push_u8(bytes, 3);
            push_u32(bytes, segment_index as u32);
        }
    }
}

fn write_ids(bytes: &mut Vec<u8>, ids: impl Iterator<Item = u64>) {
    let ids = ids.collect::<Vec<_>>();
    push_u32(bytes, ids.len() as u32);
    for id in ids {
        push_u64(bytes, id);
    }
}

fn write_features(bytes: &mut Vec<u8>, product: &ProductModel) {
    push_u32(bytes, product.features.len() as u32);
    for feature in product.features.values() {
        push_u64(bytes, feature.id().0);
        push_u64(bytes, feature.definition_id().0);
        push_string(bytes, feature.name());
        match feature.kind() {
            FeatureKind::Profile { points_mm } => {
                push_u8(bytes, 1);
                push_u32(bytes, points_mm.len() as u32);
                for point in points_mm {
                    push_u64(bytes, point[0].to_bits());
                    push_u64(bytes, point[1].to_bits());
                }
            }
            FeatureKind::Extrusion { profile, height } => {
                push_u8(bytes, 2);
                push_u64(bytes, profile.0);
                push_string(bytes, height.source_token());
                push_u64(bytes, height.millimetres().to_bits());
            }
        }
    }
}

fn write_occurrences(bytes: &mut Vec<u8>, product: &ProductModel) {
    push_u32(bytes, product.occurrences.len() as u32);
    for occurrence in product.occurrences.values() {
        push_u64(bytes, occurrence.id().0);
        push_u64(bytes, occurrence.definition_id().0);
        push_string(bytes, occurrence.name());
        push_transform(bytes, occurrence.transform());
        push_optional_id(bytes, occurrence.parent().map(|id| id.0));
        push_optional_id(bytes, occurrence.tag().map(|id| id.0));
        push_u8(bytes, u8::from(occurrence.visible()));
    }
}

fn write_groups(bytes: &mut Vec<u8>, product: &ProductModel) {
    push_u32(bytes, product.groups.len() as u32);
    for group in product.groups.values() {
        push_u64(bytes, group.id().0);
        push_string(bytes, group.name());
        push_transform(bytes, group.transform());
        push_optional_id(bytes, group.parent().map(|id| id.0));
    }
}

pub fn save_atomic(
    path: impl AsRef<Path>,
    snapshot: &Snapshot,
) -> Result<(), FilePersistenceError> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&save(snapshot))?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| FilePersistenceError::Io(error.error))?;
    Ok(())
}

pub fn load_file(path: impl AsRef<Path>) -> Result<LoadOutcome, FilePersistenceError> {
    load(&fs::read(path)?).map_err(FilePersistenceError::Format)
}

pub fn load(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(PersistenceError::InvalidMagic);
    }
    let schema = reader.u16()?;
    if !matches!(
        schema,
        LEGACY_SCHEMA | RESEARCH_SCHEMA | PRODUCT_SCHEMA | ENVELOPE_SCHEMA | CURRENT_SCHEMA
    ) {
        return Err(PersistenceError::UnsupportedSchema(schema));
    }
    let mut migration_losses = Vec::new();
    let (revision_id, product) = if matches!(schema, ENVELOPE_SCHEMA | CURRENT_SCHEMA) {
        let manifest_length = reader.count_with_limit(MAX_MANIFEST_BYTES as u32)? as usize;
        if manifest_length != MANIFEST_BYTES || bytes.len() < HEADER_BYTES + MANIFEST_BYTES {
            return Err(PersistenceError::InvalidEnvelopeLength);
        }
        let manifest_bytes = reader.take(manifest_length)?;
        let payload_length = usize::try_from(u64::from_le_bytes(
            manifest_bytes[0..8]
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
        .map_err(|_| PersistenceError::LengthOverflow)?;
        if payload_length > MAX_PAYLOAD_BYTES
            || bytes.len() != HEADER_BYTES + MANIFEST_BYTES + payload_length
        {
            return Err(PersistenceError::InvalidEnvelopeLength);
        }
        let checksum: [u8; 32] = manifest_bytes[8..40]
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        let payload = reader.take(payload_length)?;
        if crate::graph::sha256_bytes(payload) != checksum {
            return Err(PersistenceError::ChecksumMismatch);
        }
        let mut manifest = Reader::new(manifest_bytes);
        let _verified_payload_length = manifest.u64()?;
        let _verified_checksum = manifest.take(32)?;
        if manifest.string()? != crate::graph::GRAPH_SCHEMA_ID_V1
            || manifest.string()? != crate::graph::EVALUATOR_ID_V1
            || manifest.string()? != crate::document::TOLERANCE_PROFILE_V1
        {
            return Err(PersistenceError::UnsupportedEnvelopeIdentity);
        }
        if !manifest.is_finished() || !reader.is_finished() {
            return Err(PersistenceError::TrailingBytes);
        }
        let mut payload_reader = Reader::new(payload);
        let revision_id = payload_reader.u64()?;
        let product = read_product(&mut payload_reader, true, schema == CURRENT_SCHEMA)?;
        if !payload_reader.is_finished() {
            return Err(PersistenceError::TrailingBytes);
        }
        (revision_id, product)
    } else {
        let revision_id = reader.u64()?;
        let nodes = read_nodes(&mut reader, schema == LEGACY_SCHEMA, &mut migration_losses)?;
        let mut product = if schema == PRODUCT_SCHEMA {
            read_product(&mut reader, false, false)?
        } else {
            ProductModel::default()
        };
        product.evaluator_nodes = nodes;
        if !reader.is_finished() {
            return Err(PersistenceError::TrailingBytes);
        }
        (revision_id, product)
    };
    let override_health = product
        .overrides
        .values()
        .map(|value| OverrideHealthAudit {
            override_id: value.id,
            stored: value.health.clone(),
            audited: crate::graph::resolve_derived_identity(
                &product.evaluator_nodes,
                &value.target,
            ),
        })
        .collect::<Vec<_>>();
    let review_required = !migration_losses.is_empty()
        || override_health.iter().any(|entry| {
            entry.audited != SlotResolution::Resolved || entry.audited != entry.stored
        });
    let audit = LoadAudit {
        source_schema: schema,
        migration_losses,
        override_health,
    };
    let document = DocumentStore::from_product(revision_id, product)?;
    let loaded_snapshot = document.current();
    for reference in loaded_snapshot.exact_reference_evidence() {
        let request = crate::exact_product::ExactRectangleRequest::from_snapshot(
            &loaded_snapshot,
            reference.definition_id,
        )
        .map_err(|_| PersistenceError::InvalidExactReference)?;
        if request.profile_feature_id != reference.profile_feature_id
            || request.extrusion_feature_id != reference.producer_feature_id
            || request.canonical_input_digest != reference.canonical_input_digest
        {
            return Err(PersistenceError::InvalidExactReference);
        }
    }
    if review_required {
        Ok(LoadOutcome::ReviewOnly(ReviewCandidate {
            snapshot: document.current(),
            audit,
        }))
    } else {
        Ok(LoadOutcome::Editable { document, audit })
    }
}

fn read_nodes(
    reader: &mut Reader<'_>,
    legacy: bool,
    migration_losses: &mut Vec<MigrationLoss>,
) -> Result<BTreeMap<NodeId, Arc<EvaluatorNode>>, PersistenceError> {
    let mut nodes = BTreeMap::new();
    for _ in 0..reader.count()? {
        let id = NodeId(reader.u64()?);
        let name = reader.string()?;
        let stored_token = if legacy {
            String::new()
        } else {
            reader.string()?
        };
        let millimetres = f64::from_bits(reader.u64()?);
        let source_token = if legacy {
            migration_losses.push(MigrationLoss {
                node_id: id,
                field: "dimension.source_token",
                reason: "legacy schema stored only the canonical binary value",
            });
            format!("{millimetres:.17}")
        } else {
            stored_token
        };
        let dependencies = read_ids(reader)?.into_iter().map(NodeId).collect();
        let node = EvaluatorNode::parameter(
            id,
            name,
            Dimension::new(source_token, millimetres)?,
            dependencies,
        )
        .map_err(CanonicalError::Graph)?;
        if nodes.insert(id, Arc::new(node)).is_some() {
            return Err(PersistenceError::DuplicateNode(id));
        }
    }
    Ok(nodes)
}

fn read_current_nodes(
    reader: &mut Reader<'_>,
) -> Result<BTreeMap<NodeId, Arc<EvaluatorNode>>, PersistenceError> {
    let mut nodes = BTreeMap::new();
    for _ in 0..reader.count()? {
        let id = NodeId(reader.u64()?);
        let name = reader.string()?;
        let node = match reader.u8()? {
            1 => EvaluatorNode::parameter(
                id,
                name,
                Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                read_ids(reader)?.into_iter().map(NodeId).collect(),
            ),
            2 => EvaluatorNode::expression(id, name, reader.string()?),
            3 => EvaluatorNode::rule(
                id,
                name,
                reader.string()?,
                read_ports(reader)?,
                read_ports(reader)?,
                read_rule_outputs(reader)?,
                read_override_parameters(reader)?,
            ),
            kind => return Err(PersistenceError::InvalidNodeKind(kind)),
        }
        .map_err(CanonicalError::Graph)?;
        if nodes.insert(id, Arc::new(node)).is_some() {
            return Err(PersistenceError::DuplicateNode(id));
        }
    }
    Ok(nodes)
}

fn read_ports(reader: &mut Reader<'_>) -> Result<Vec<PortSpec>, PersistenceError> {
    let mut ports = Vec::new();
    for _ in 0..reader.count()? {
        let name = reader.string()?;
        if reader.u8()? != 1 {
            return Err(PersistenceError::InvalidPortType);
        }
        ports.push(PortSpec::number(name).map_err(CanonicalError::Graph)?);
    }
    Ok(ports)
}

fn read_override_parameters(
    reader: &mut Reader<'_>,
) -> Result<Vec<OverrideParameterSpec>, PersistenceError> {
    let mut parameters = Vec::new();
    for _ in 0..reader.count()? {
        let name = reader.string()?;
        if reader.u8()? != 1 {
            return Err(PersistenceError::InvalidOverrideMergePolicy);
        }
        parameters.push(OverrideParameterSpec::replace(name).map_err(CanonicalError::Graph)?);
    }
    Ok(parameters)
}

fn read_rule_outputs(reader: &mut Reader<'_>) -> Result<Vec<RuleOutput>, PersistenceError> {
    struct Frame {
        remaining: u32,
        outputs: Vec<RuleOutput>,
        parent: Option<SlotSegment>,
    }
    let root_count = reader.count()?;
    let mut root_outputs = Vec::new();
    root_outputs
        .try_reserve_exact(root_count as usize)
        .map_err(|_| PersistenceError::ResourceLimit)?;
    let mut frames = vec![Frame {
        remaining: root_count,
        outputs: root_outputs,
        parent: None,
    }];
    loop {
        let frame = frames.last_mut().ok_or(PersistenceError::Truncated)?;
        if frame.remaining == 0 {
            let completed = frames.pop().ok_or(PersistenceError::Truncated)?;
            if let Some(segment) = completed.parent {
                let output =
                    RuleOutput::new(segment, completed.outputs).map_err(CanonicalError::Graph)?;
                frames
                    .last_mut()
                    .ok_or(PersistenceError::Truncated)?
                    .outputs
                    .push(output);
                continue;
            }
            return Ok(completed.outputs);
        }
        frame.remaining -= 1;
        let segment = SlotSegment::new(NodeId(reader.u64()?), reader.string()?, reader.string()?)
            .map_err(CanonicalError::Graph)?;
        let child_count = reader.count()?;
        if frames.len() >= crate::graph::MAX_RULE_OUTPUT_DEPTH {
            return Err(PersistenceError::ResourceLimit);
        }
        let mut children = Vec::new();
        children
            .try_reserve_exact(child_count as usize)
            .map_err(|_| PersistenceError::ResourceLimit)?;
        frames.push(Frame {
            remaining: child_count,
            outputs: children,
            parent: Some(segment),
        });
    }
}

fn read_slot_path(reader: &mut Reader<'_>) -> Result<SlotPath, PersistenceError> {
    let mut segments = Vec::new();
    for _ in 0..reader.count()? {
        segments.push(
            SlotSegment::new(NodeId(reader.u64()?), reader.string()?, reader.string()?)
                .map_err(CanonicalError::Graph)?,
        );
    }
    SlotPath::new(segments)
        .map_err(|error| PersistenceError::InvalidCanonicalData(CanonicalError::Graph(error)))
}

fn read_identity(reader: &mut Reader<'_>) -> Result<DerivedIdentity, PersistenceError> {
    DerivedIdentity::new(NodeId(reader.u64()?), read_slot_path(reader)?)
        .map_err(CanonicalError::Graph)
        .map_err(PersistenceError::from)
}

fn read_joint(reader: &mut Reader<'_>) -> Result<CanonicalJoint, PersistenceError> {
    let id = JointId(reader.u64()?);
    let participant_a = read_identity(reader)?;
    let participant_b = read_identity(reader)?;
    let min = [
        f64::from_bits(reader.u64()?),
        f64::from_bits(reader.u64()?),
        f64::from_bits(reader.u64()?),
    ];
    let max = [
        f64::from_bits(reader.u64()?),
        f64::from_bits(reader.u64()?),
        f64::from_bits(reader.u64()?),
    ];
    let volume = Aabb::bounded_volume(min, max).map_err(CanonicalError::from)?;
    CanonicalJoint::new(id, participant_a, participant_b, volume)
        .map_err(CanonicalError::from)
        .map_err(PersistenceError::from)
}

fn read_override(reader: &mut Reader<'_>) -> Result<CanonicalOverride, PersistenceError> {
    let id = reader.u64()?;
    let target = DerivedIdentity::new(NodeId(reader.u64()?), read_slot_path(reader)?)
        .map_err(CanonicalError::Graph)?;
    let parameter = reader.string()?;
    let value = f64::from_bits(reader.u64()?);
    let health = match reader.u8()? {
        1 => SlotResolution::Resolved,
        2 => SlotResolution::Ambiguous {
            segment_index: reader.count()? as usize,
        },
        3 => SlotResolution::Lost {
            segment_index: reader.count()? as usize,
        },
        value => return Err(PersistenceError::InvalidResolution(value)),
    };
    CanonicalOverride::new(id, target, parameter, value, health)
        .map_err(|error| PersistenceError::InvalidCanonicalData(CanonicalError::Graph(error)))
}

fn read_ids(reader: &mut Reader<'_>) -> Result<Vec<u64>, PersistenceError> {
    let mut ids = Vec::new();
    for _ in 0..reader.count()? {
        ids.push(reader.u64()?);
    }
    Ok(ids)
}

fn read_exact_reference(reader: &mut Reader<'_>) -> Result<BodySubshapeRef, PersistenceError> {
    let reference = BodySubshapeRef {
        schema: reader.string()?,
        document_id: crate::document::DocumentId(reader.u64()?),
        definition_id: DefinitionId(reader.u64()?),
        profile_feature_id: FeatureId(reader.u64()?),
        producer_feature_id: FeatureId(reader.u64()?),
        semantic_role: reader.string()?,
        source_element_id: reader.string()?,
        expected_type: reader.string()?,
        expected_cardinality: reader.count()?,
        stability: match reader.u8()? {
            1 => ReferenceStability::Guaranteed,
            value => return Err(PersistenceError::InvalidReferenceStability(value)),
        },
        canonical_input_digest: reader.string()?,
        exact_input_digest: reader.string()?,
        result_fingerprint: reader.string()?,
        evaluator: reader.string()?,
        backend: reader.string()?,
        tolerance: reader.string()?,
        lineage_digest: reader.string()?,
        corroborating_geometry_fingerprint: reader.string()?,
    };
    if reference.schema != BODY_SUBSHAPE_REF_SCHEMA_V1 || !reference.has_valid_lineage() {
        return Err(PersistenceError::InvalidExactReference);
    }
    Ok(reference)
}

fn read_product(
    reader: &mut Reader<'_>,
    current: bool,
    exact_evidence: bool,
) -> Result<ProductModel, PersistenceError> {
    let mut product = ProductModel {
        document_id: crate::document::DocumentId(reader.u64()?),
        units: match reader.u8()? {
            1 => UnitSystem::Millimetres,
            units => return Err(PersistenceError::UnsupportedUnits(units)),
        },
        ..ProductModel::default()
    };
    if current {
        product.evaluator_nodes = read_current_nodes(reader)?;
        for _ in 0..reader.count()? {
            let value = read_override(reader)?;
            if product
                .overrides
                .insert(value.id, Arc::new(value))
                .is_some()
            {
                return Err(PersistenceError::DuplicateOverride);
            }
        }
    }
    for _ in 0..reader.count()? {
        let id = DefinitionId(reader.u64()?);
        let definition = Definition {
            id,
            name: reader.string()?,
            feature_ids: read_ids(reader)?.into_iter().map(FeatureId).collect(),
            local_group_ids: if current {
                read_ids(reader)?.into_iter().map(LocalGroupId).collect()
            } else {
                Vec::new()
            },
            local_occurrence_ids: if current {
                read_ids(reader)?
                    .into_iter()
                    .map(LocalOccurrenceId)
                    .collect()
            } else {
                Vec::new()
            },
        };
        if product
            .definitions
            .insert(id, Arc::new(definition))
            .is_some()
        {
            return Err(PersistenceError::DuplicateDefinition(id));
        }
    }
    for _ in 0..reader.count()? {
        let id = FeatureId(reader.u64()?);
        let definition_id = DefinitionId(reader.u64()?);
        let name = reader.string()?;
        let kind = match reader.u8()? {
            1 => {
                let mut points_mm = Vec::new();
                for _ in 0..reader.count()? {
                    points_mm.push([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)]);
                }
                FeatureKind::Profile { points_mm }
            }
            2 => FeatureKind::Extrusion {
                profile: FeatureId(reader.u64()?),
                height: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            kind => return Err(PersistenceError::InvalidFeatureKind(kind)),
        };
        let feature = Feature {
            id,
            definition_id,
            name,
            kind,
        };
        if product.features.insert(id, Arc::new(feature)).is_some() {
            return Err(PersistenceError::DuplicateFeature(id));
        }
    }
    for _ in 0..reader.count()? {
        let id = OccurrenceId(reader.u64()?);
        let occurrence = Occurrence {
            id,
            definition_id: DefinitionId(reader.u64()?),
            name: reader.string()?,
            transform: reader.transform()?,
            parent: reader.optional_id()?.map(GroupId),
            tag: reader.optional_id()?.map(TagId),
            visible: reader.boolean()?,
        };
        if product
            .occurrences
            .insert(id, Arc::new(occurrence))
            .is_some()
        {
            return Err(PersistenceError::DuplicateOccurrence(id));
        }
    }
    for _ in 0..reader.count()? {
        let id = GroupId(reader.u64()?);
        let group = Group {
            id,
            name: reader.string()?,
            transform: reader.transform()?,
            parent: reader.optional_id()?.map(GroupId),
        };
        if product.groups.insert(id, Arc::new(group)).is_some() {
            return Err(PersistenceError::DuplicateGroup(id));
        }
    }
    if current {
        for _ in 0..reader.count()? {
            let key = LocalGroupKey {
                definition_id: DefinitionId(reader.u64()?),
                local_id: LocalGroupId(reader.u64()?),
            };
            let group = LocalGroup {
                key,
                name: reader.string()?,
                transform: reader.transform()?,
                parent: reader.optional_id()?.map(LocalGroupId),
            };
            if product.local_groups.insert(key, Arc::new(group)).is_some() {
                return Err(PersistenceError::DuplicateLocalGroup(key));
            }
        }
        for _ in 0..reader.count()? {
            let key = LocalOccurrenceKey {
                definition_id: DefinitionId(reader.u64()?),
                local_id: LocalOccurrenceId(reader.u64()?),
            };
            let occurrence = LocalOccurrence {
                key,
                definition_id: DefinitionId(reader.u64()?),
                name: reader.string()?,
                transform: reader.transform()?,
                parent: reader.optional_id()?.map(LocalGroupId),
                tag: reader.optional_id()?.map(TagId),
                visible: reader.boolean()?,
            };
            if product
                .local_occurrences
                .insert(key, Arc::new(occurrence))
                .is_some()
            {
                return Err(PersistenceError::DuplicateLocalOccurrence(key));
            }
        }
        if !reader.is_finished() {
            for _ in 0..reader.count()? {
                let joint = read_joint(reader)?;
                if product.joints.insert(joint.id(), Arc::new(joint)).is_some() {
                    return Err(PersistenceError::DuplicateJoint);
                }
            }
        }
        if exact_evidence {
            for _ in 0..reader.count()? {
                let reference = read_exact_reference(reader)?;
                let producer = product
                    .features
                    .get(&reference.producer_feature_id)
                    .ok_or(PersistenceError::InvalidExactReference)?;
                if reference.document_id != product.document_id
                    || producer.definition_id != reference.definition_id
                {
                    return Err(PersistenceError::InvalidExactReference);
                }
                if product
                    .exact_reference_evidence
                    .insert(reference.lineage_digest.clone(), Arc::new(reference))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateExactReference);
                }
            }
        }
    }
    Ok(product)
}

#[derive(Debug)]
pub enum FilePersistenceError {
    Io(io::Error),
    Format(PersistenceError),
}

impl fmt::Display for FilePersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Format(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FilePersistenceError {}
impl From<io::Error> for FilePersistenceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PersistenceError {
    Truncated,
    InvalidMagic,
    UnsupportedSchema(u16),
    InvalidUtf8,
    LengthOverflow,
    TrailingBytes,
    InvalidEnvelopeLength,
    InvalidBoolean(u8),
    UnsupportedUnits(u8),
    InvalidFeatureKind(u8),
    InvalidNodeKind(u8),
    InvalidPortType,
    InvalidOverrideMergePolicy,
    InvalidResolution(u8),
    InvalidReferenceStability(u8),
    InvalidExactReference,
    ChecksumMismatch,
    ResourceLimit,
    UnsupportedEnvelopeIdentity,
    DuplicateOverride,
    DuplicateJoint,
    DuplicateExactReference,
    DuplicateNode(NodeId),
    DuplicateDefinition(DefinitionId),
    DuplicateFeature(FeatureId),
    DuplicateOccurrence(OccurrenceId),
    DuplicateGroup(GroupId),
    DuplicateLocalGroup(LocalGroupKey),
    DuplicateLocalOccurrence(LocalOccurrenceKey),
    InvalidCanonicalData(CanonicalError),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("document is truncated"),
            Self::InvalidMagic => formatter.write_str("document magic is invalid"),
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "document schema {schema} is unsupported")
            }
            Self::InvalidUtf8 => formatter.write_str("document string is not UTF-8"),
            Self::LengthOverflow => formatter.write_str("document length exceeds this platform"),
            Self::TrailingBytes => formatter.write_str("document has trailing bytes"),
            Self::InvalidEnvelopeLength => {
                formatter.write_str("document envelope length is invalid")
            }
            Self::InvalidBoolean(value) => write!(formatter, "document boolean {value} is invalid"),
            Self::UnsupportedUnits(units) => {
                write!(formatter, "document unit system {units} is unsupported")
            }
            Self::InvalidFeatureKind(kind) => write!(formatter, "feature kind {kind} is invalid"),
            Self::InvalidNodeKind(kind) => write!(formatter, "node kind {kind} is invalid"),
            Self::InvalidPortType => formatter.write_str("typed port kind is invalid"),
            Self::InvalidOverrideMergePolicy => {
                formatter.write_str("override merge policy is invalid")
            }
            Self::InvalidResolution(value) => {
                write!(formatter, "slot resolution {value} is invalid")
            }
            Self::InvalidReferenceStability(value) => {
                write!(formatter, "exact reference stability {value} is invalid")
            }
            Self::InvalidExactReference => {
                formatter.write_str("exact reference evidence is invalid")
            }
            Self::ChecksumMismatch => formatter.write_str("document checksum does not match"),
            Self::ResourceLimit => formatter.write_str("document exceeds a resource limit"),
            Self::UnsupportedEnvelopeIdentity => {
                formatter.write_str("document envelope identity is unsupported")
            }
            Self::DuplicateOverride => formatter.write_str("document repeats an override"),
            Self::DuplicateJoint => formatter.write_str("document repeats a joint"),
            Self::DuplicateExactReference => {
                formatter.write_str("document repeats exact reference evidence")
            }
            Self::DuplicateNode(id) => write!(formatter, "document repeats node {}", id.0),
            Self::DuplicateDefinition(id) => {
                write!(formatter, "document repeats definition {}", id.0)
            }
            Self::DuplicateFeature(id) => write!(formatter, "document repeats feature {}", id.0),
            Self::DuplicateOccurrence(id) => {
                write!(formatter, "document repeats occurrence {}", id.0)
            }
            Self::DuplicateGroup(id) => write!(formatter, "document repeats group {}", id.0),
            Self::DuplicateLocalGroup(key) => write!(
                formatter,
                "document repeats local group {}:{}",
                key.definition_id.0, key.local_id.0
            ),
            Self::DuplicateLocalOccurrence(key) => write!(
                formatter,
                "document repeats local occurrence {}:{}",
                key.definition_id.0, key.local_id.0
            ),
            Self::InvalidCanonicalData(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PersistenceError {}
impl From<CanonicalError> for PersistenceError {
    fn from(error: CanonicalError) -> Self {
        Self::InvalidCanonicalData(error)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
    collection_items: u64,
    string_bytes: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            cursor: 0,
            collection_items: 0,
            string_bytes: 0,
        }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], PersistenceError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(PersistenceError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(PersistenceError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, PersistenceError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, PersistenceError> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, PersistenceError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, PersistenceError> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| PersistenceError::Truncated)?,
        ))
    }
    fn count(&mut self) -> Result<u32, PersistenceError> {
        self.count_with_limit(MAX_COLLECTION_ITEMS)
    }
    fn count_with_limit(&mut self, limit: u32) -> Result<u32, PersistenceError> {
        let value = self.u32()?;
        self.collection_items = self
            .collection_items
            .checked_add(u64::from(value))
            .ok_or(PersistenceError::ResourceLimit)?;
        if value > limit || self.collection_items > u64::from(MAX_COLLECTION_ITEMS) {
            Err(PersistenceError::ResourceLimit)
        } else {
            Ok(value)
        }
    }
    fn string(&mut self) -> Result<String, PersistenceError> {
        let length = usize::try_from(self.count_with_limit(MAX_STRING_BYTES as u32)?)
            .map_err(|_| PersistenceError::LengthOverflow)?;
        self.string_bytes = self
            .string_bytes
            .checked_add(length)
            .ok_or(PersistenceError::ResourceLimit)?;
        if self.string_bytes > MAX_STRING_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        let value =
            std::str::from_utf8(self.take(length)?).map_err(|_| PersistenceError::InvalidUtf8)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(length)
            .map_err(|_| PersistenceError::ResourceLimit)?;
        owned.push_str(value);
        Ok(owned)
    }
    fn optional_id(&mut self) -> Result<Option<u64>, PersistenceError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            value => Err(PersistenceError::InvalidBoolean(value)),
        }
    }
    fn boolean(&mut self) -> Result<bool, PersistenceError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(PersistenceError::InvalidBoolean(value)),
        }
    }
    fn transform(&mut self) -> Result<Transform, PersistenceError> {
        let mut matrix = [0.0; 16];
        for value in &mut matrix {
            *value = f64::from_bits(self.u64()?);
        }
        Ok(Transform::from_matrix(matrix)?)
    }
    const fn is_finished(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn push_u8(bytes: &mut Vec<u8>, value: u8) {
    bytes.push(value);
}
fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn push_string(bytes: &mut Vec<u8>, value: &str) {
    push_u32(bytes, value.len() as u32);
    bytes.extend_from_slice(value.as_bytes());
}
fn push_transform(bytes: &mut Vec<u8>, transform: Transform) {
    for value in transform.matrix() {
        push_u64(bytes, value.to_bits());
    }
}
fn push_optional_id(bytes: &mut Vec<u8>, id: Option<u64>) {
    match id {
        Some(id) => {
            push_u8(bytes, 1);
            push_u64(bytes, id);
        }
        None => push_u8(bytes, 0),
    }
}
