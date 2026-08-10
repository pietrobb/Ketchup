use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::document::{
    BooleanOperation, BottleEdgeFinishKind, CanonicalCommand, CanonicalError, Collection,
    CollectionId, CommandBatch, Definition, DefinitionId, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DocumentStore, EvaluationIdentity, EvaluatorNode,
    ExactReferenceConversionConsequence, ExactToMeshConversion, Feature, FeatureId, FeatureKind,
    FeatureParameterBinding, FeatureParameterFreshnessAudit, FeatureParameterProvenance,
    FeatureParameterSlot, FeatureParameterTarget, Group, GroupId, InstancePath, InstancePathStep,
    LocalGroup, LocalGroupId, LocalGroupKey, LocalOccurrence, LocalOccurrenceId,
    LocalOccurrenceKey, MeshAuthority, MeshBodySpec, NodeId, Occurrence, OccurrenceId,
    PersistentDimension, PersistentDimensionId, PersistentDimensionTarget, ProductModel, Snapshot,
    Tag, TagId, Transform, UnitSystem,
};
use crate::exact_product::{BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ReferenceStability};
use crate::graph::{
    CanonicalOverride, DerivedIdentity, EvaluatorNodeKind, OverrideMergePolicy,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment,
};
use crate::prismatic::{Aabb, CanonicalJoint, JointId, TolerancePolicy};
use crate::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceCoordinateFrame, ClearanceOwner,
    ClearanceSeverity, ClearanceVolumeId, SpaceId,
};

const MAGIC: &[u8; 10] = b"KETCHUPDOC";
const CONTAINER_MAGIC: &[u8; 10] = b"KETCHUPCTR";
const CONTAINER_SCHEMA: u16 = 1;
const MESH_BODY_SCHEMA: u16 = 16;
const SPACE_CLEARANCE_SCHEMA: u16 = 17;
const CURRENT_SCHEMA: u16 = SPACE_CLEARANCE_SCHEMA;
const COLLECTION_SCHEMA: u16 = 15;
const TAG_SCHEMA: u16 = 14;
const PERSISTENT_DIMENSION_SCHEMA: u16 = 13;
const PROFILE_CONSTRAINT_SCHEMA: u16 = 12;
const PARAMETRIC_PROVENANCE_SCHEMA: u16 = 11;
const PARAMETRIC_BINDING_SCHEMA: u16 = 10;
const BOOLEAN_SCHEMA: u16 = 9;
const BOTTLE_FINISH_SCHEMA: u16 = 8;
const SHELL_SCHEMA: u16 = 7;
const REVOLVE_SCHEMA: u16 = 6;
const THROUGH_CUT_SCHEMA: u16 = 5;
const EXACT_EVIDENCE_SCHEMA: u16 = 4;
const ENVELOPE_SCHEMA: u16 = 3;
const PRODUCT_SCHEMA: u16 = 2;
const RESEARCH_SCHEMA: u16 = 1;
const LEGACY_SCHEMA: u16 = 0;

#[derive(Clone, Copy)]
struct ProductSchemaCapabilities {
    current: bool,
    exact_evidence: bool,
    through_cut: bool,
    revolve: bool,
    shell: bool,
    bottle_finish: bool,
    boolean: bool,
    parametric_bindings: bool,
    parametric_provenance: bool,
    persistent_dimensions: bool,
    tags: bool,
    collections: bool,
    mesh_body: bool,
    space_clearance: bool,
}

impl ProductSchemaCapabilities {
    const PRODUCT_V2: Self = Self {
        current: false,
        exact_evidence: false,
        through_cut: false,
        revolve: false,
        shell: false,
        bottle_finish: false,
        boolean: false,
        parametric_bindings: false,
        parametric_provenance: false,
        persistent_dimensions: false,
        tags: false,
        collections: false,
        mesh_body: false,
        space_clearance: false,
    };

    const fn current(schema: u16) -> Self {
        Self {
            current: schema >= THROUGH_CUT_SCHEMA,
            exact_evidence: schema >= EXACT_EVIDENCE_SCHEMA,
            through_cut: schema >= THROUGH_CUT_SCHEMA,
            revolve: schema >= REVOLVE_SCHEMA,
            shell: schema >= SHELL_SCHEMA,
            bottle_finish: schema >= BOTTLE_FINISH_SCHEMA,
            boolean: schema >= BOOLEAN_SCHEMA,
            parametric_bindings: schema >= PARAMETRIC_BINDING_SCHEMA,
            parametric_provenance: schema >= PARAMETRIC_PROVENANCE_SCHEMA,
            persistent_dimensions: schema >= PERSISTENT_DIMENSION_SCHEMA,
            tags: schema >= TAG_SCHEMA,
            collections: schema >= COLLECTION_SCHEMA,
            mesh_body: schema >= MESH_BODY_SCHEMA,
            space_clearance: schema >= SPACE_CLEARANCE_SCHEMA,
        }
    }
}
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
const MAX_COLLECTION_ITEMS: u32 = 500_000;
const MAX_CONTAINER_BYTES: usize = 64 * 1024 * 1024;
const MAX_CONTAINER_ENTRIES: u32 = 4_096;
const MAX_CONTAINER_PATH_BYTES: usize = 1_024;
const MAX_SIDECAR_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionEntry {
    namespace: String,
    path: String,
    required: bool,
    bytes: Vec<u8>,
}

impl ExtensionEntry {
    pub fn new(
        namespace: impl Into<String>,
        path: impl Into<String>,
        required: bool,
        bytes: Vec<u8>,
    ) -> Result<Self, PersistenceError> {
        let namespace = namespace.into();
        let path = path.into();
        validate_namespace(&namespace)?;
        validate_relative_path(&path)?;
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        Ok(Self {
            namespace,
            path,
            required,
            bytes,
        })
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContainerData {
    blobs: BTreeMap<String, Vec<u8>>,
    extensions: BTreeMap<(String, String), ExtensionEntry>,
}

impl ContainerData {
    pub fn insert_blob(&mut self, bytes: Vec<u8>) -> Result<String, PersistenceError> {
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
        let hash = crate::graph::sha256_hex(&bytes);
        self.blobs.entry(hash.clone()).or_insert(bytes);
        Ok(hash)
    }

    pub fn insert_extension(&mut self, entry: ExtensionEntry) -> Result<(), PersistenceError> {
        let key = (entry.namespace.clone(), entry.path.clone());
        if self.extensions.insert(key, entry).is_some() {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
        Ok(())
    }

    #[must_use]
    pub fn blobs(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.blobs
    }

    pub fn extensions(&self) -> impl Iterator<Item = &ExtensionEntry> {
        self.extensions.values()
    }

    #[must_use]
    pub fn requires_unknown_extension(&self) -> bool {
        self.extensions.values().any(ExtensionEntry::required)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionAudit {
    pub namespace: String,
    pub path: String,
    pub required: bool,
}

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
    pub feature_parameter_freshness: Vec<FeatureParameterFreshnessAudit>,
    pub unknown_extensions: Vec<ExtensionAudit>,
    pub recovered_from_backup: bool,
}

pub struct ReviewCandidate {
    snapshot: Snapshot,
    audit: Box<LoadAudit>,
    container_data: Box<ContainerData>,
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
    #[must_use]
    pub const fn container_data(&self) -> &ContainerData {
        &self.container_data
    }

    pub fn confirm_semantic_migration(&self) -> Result<ConfirmedMigration, PersistenceError> {
        if self.container_data.requires_unknown_extension() {
            return Err(PersistenceError::MigrationNotConfirmable(
                "candidate requires an unknown extension",
            ));
        }
        if self.audit.migration_losses.is_empty() {
            return Err(PersistenceError::MigrationNotConfirmable(
                "candidate has no reported semantic loss",
            ));
        }
        if self
            .audit
            .override_health
            .iter()
            .any(|entry| entry.audited != SlotResolution::Resolved || entry.audited != entry.stored)
        {
            return Err(PersistenceError::MigrationNotConfirmable(
                "candidate has unresolved override identity",
            ));
        }

        let mut migrated_nodes = BTreeSet::new();
        let mut commands = Vec::new();
        for loss in &self.audit.migration_losses {
            if loss.field != "dimension.source_token" || !migrated_nodes.insert(loss.node_id) {
                return Err(PersistenceError::MigrationNotConfirmable(
                    "candidate contains an unsupported migration loss",
                ));
            }
            let dimension = self
                .snapshot
                .evaluator_node(loss.node_id)
                .and_then(EvaluatorNode::dimension)
                .cloned()
                .ok_or(PersistenceError::MigrationNotConfirmable(
                    "migration target is not a parameter dimension",
                ))?;
            commands.push(CanonicalCommand::SetEvaluatorDimension {
                id: loss.node_id,
                dimension,
            });
        }

        let source_revision_id = self.snapshot.revision_id();
        let mut document =
            DocumentStore::from_product(source_revision_id, self.snapshot.product().clone())?;
        let confirmed = document.apply_batch(&CommandBatch::new(commands))?;
        Ok(ConfirmedMigration {
            document,
            container_data: self.container_data.as_ref().clone(),
            source_schema: self.audit.source_schema,
            source_revision_id,
            confirmed_revision_id: confirmed.id(),
            losses: self.audit.migration_losses.clone(),
        })
    }
}

pub struct ConfirmedMigration {
    document: DocumentStore,
    container_data: ContainerData,
    source_schema: u16,
    source_revision_id: u64,
    confirmed_revision_id: u64,
    losses: Vec<MigrationLoss>,
}

impl ConfirmedMigration {
    #[must_use]
    pub const fn source_schema(&self) -> u16 {
        self.source_schema
    }

    #[must_use]
    pub const fn source_revision_id(&self) -> u64 {
        self.source_revision_id
    }

    #[must_use]
    pub const fn confirmed_revision_id(&self) -> u64 {
        self.confirmed_revision_id
    }

    #[must_use]
    pub fn losses(&self) -> &[MigrationLoss] {
        &self.losses
    }

    pub fn into_parts(self) -> (DocumentStore, ContainerData) {
        (self.document, self.container_data)
    }
}

pub enum LoadOutcome {
    Editable {
        document: DocumentStore,
        audit: LoadAudit,
        container_data: ContainerData,
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
        self.into_editable_with_container()
            .map(|(document, _)| document)
    }
    pub fn into_editable_with_container(
        self,
    ) -> Result<(DocumentStore, ContainerData), ReviewCandidate> {
        match self {
            Self::Editable {
                document,
                container_data,
                ..
            } => Ok((document, container_data)),
            Self::ReviewOnly(candidate) => Err(candidate),
        }
    }
    #[must_use]
    pub const fn container_data(&self) -> &ContainerData {
        match self {
            Self::Editable { container_data, .. } => container_data,
            Self::ReviewOnly(candidate) => candidate.container_data(),
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
    push_u32(
        &mut payload,
        product.feature_parameter_bindings.len() as u32,
    );
    for binding in product.feature_parameter_bindings.values() {
        write_feature_parameter_binding(&mut payload, binding);
        if let Some(provenance) = product.feature_parameter_provenance.get(&binding.target) {
            push_u8(&mut payload, 1);
            write_feature_parameter_provenance(&mut payload, provenance);
        } else {
            push_u8(&mut payload, 0);
        }
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
    push_u32(&mut payload, product.persistent_dimensions.len() as u32);
    for dimension in product.persistent_dimensions.values() {
        write_persistent_dimension(&mut payload, dimension);
    }
    push_u32(&mut payload, product.tags.len() as u32);
    for tag in product.tags.values() {
        push_u64(&mut payload, tag.id().0);
        push_string(&mut payload, tag.name());
        push_u8(&mut payload, u8::from(tag.visible()));
    }
    push_u32(&mut payload, product.collections.len() as u32);
    for collection in product.collections.values() {
        push_u64(&mut payload, collection.id().0);
        push_string(&mut payload, collection.name());
        write_ids(&mut payload, collection.occurrence_ids().map(|id| id.0));
    }
    push_u32(&mut payload, product.spaces.len() as u32);
    for space in product.spaces.values() {
        write_space(&mut payload, space);
    }
    push_u32(&mut payload, product.clearance_volumes.len() as u32);
    for clearance in product.clearance_volumes.values() {
        write_clearance_volume(&mut payload, clearance);
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

pub fn save_container(
    snapshot: &Snapshot,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    let mut entries = BTreeMap::<String, (bool, Vec<u8>)>::new();
    entries.insert("document.bin".to_owned(), (true, save(snapshot)));
    for (hash, bytes) in &container_data.blobs {
        if crate::graph::sha256_hex(bytes) != *hash {
            return Err(PersistenceError::InvalidBlobHash);
        }
        entries.insert(format!("blobs/{hash}"), (false, bytes.clone()));
    }
    for extension in container_data.extensions.values() {
        let path = format!("extensions/{}/{}", extension.namespace, extension.path);
        if entries
            .insert(path, (extension.required, extension.bytes.clone()))
            .is_some()
        {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
    }
    if entries.len() > MAX_CONTAINER_ENTRIES as usize {
        return Err(PersistenceError::ResourceLimit);
    }

    let mut encoded = Vec::new();
    encoded.extend_from_slice(CONTAINER_MAGIC);
    push_u16(&mut encoded, CONTAINER_SCHEMA);
    push_u32(&mut encoded, entries.len() as u32);
    for (path, (required, bytes)) in entries {
        validate_container_path(&path)?;
        if bytes.len() > MAX_SIDECAR_BYTES && path != "document.bin" {
            return Err(PersistenceError::ResourceLimit);
        }
        push_string(&mut encoded, &path);
        push_u8(&mut encoded, u8::from(required));
        push_u64(&mut encoded, bytes.len() as u64);
        encoded.extend_from_slice(&crate::graph::sha256_bytes(&bytes));
        encoded.extend_from_slice(&bytes);
        if encoded.len() > MAX_CONTAINER_BYTES {
            return Err(PersistenceError::ResourceLimit);
        }
    }
    Ok(encoded)
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

fn write_space(bytes: &mut Vec<u8>, value: &CanonicalSpace) {
    push_u64(bytes, value.id().0);
    push_string(bytes, value.purpose());
    for coordinate in value.volume().min().into_iter().chain(value.volume().max()) {
        push_u64(bytes, coordinate.to_bits());
    }
    write_ids(bytes, value.adjacent_to().iter().map(|id| id.0));
    write_ids(bytes, value.accessible_to().iter().map(|id| id.0));
}

fn write_clearance_volume(bytes: &mut Vec<u8>, value: &CanonicalClearanceVolume) {
    push_u64(bytes, value.id().0);
    match value.owner() {
        ClearanceOwner::Occurrence(path) => {
            push_u8(bytes, 1);
            push_u64(bytes, path.root_occurrence().0);
            push_u32(bytes, path.steps().len() as u32);
            for step in path.steps() {
                match step {
                    InstancePathStep::Group(id) => {
                        push_u8(bytes, 1);
                        push_u64(bytes, id.0);
                    }
                    InstancePathStep::Occurrence(id) => {
                        push_u8(bytes, 2);
                        push_u64(bytes, id.0);
                    }
                }
            }
        }
        ClearanceOwner::Space(id) => {
            push_u8(bytes, 2);
            push_u64(bytes, id.0);
        }
    }
    push_string(bytes, value.reason());
    for coordinate in value.volume().min().into_iter().chain(value.volume().max()) {
        push_u64(bytes, coordinate.to_bits());
    }
    push_u8(
        bytes,
        match value.coordinate_frame() {
            ClearanceCoordinateFrame::World => 1,
        },
    );
    push_u64(bytes, value.tolerance().epsilon_mm().to_bits());
    push_u8(
        bytes,
        match value.severity() {
            ClearanceSeverity::Advisory => 1,
            ClearanceSeverity::Required => 2,
        },
    );
    match value.derived_from() {
        Some(identity) => {
            push_u8(bytes, 1);
            write_identity(bytes, identity);
        }
        None => push_u8(bytes, 0),
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

fn write_persistent_dimension(bytes: &mut Vec<u8>, dimension: &PersistentDimension) {
    push_u64(bytes, dimension.id.0);
    push_string(bytes, &dimension.name);
    match &dimension.target {
        PersistentDimensionTarget::FeatureParameter(target) => {
            push_u8(bytes, 1);
            push_u64(bytes, target.feature_id.0);
            write_feature_parameter_slot(bytes, target.slot);
        }
        PersistentDimensionTarget::DerivedOutput(target) => {
            push_u8(bytes, 2);
            write_identity(bytes, target);
        }
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id,
            producer_feature_id,
            semantic_role,
            source_element_id,
            slot,
        } => {
            push_u8(bytes, 3);
            push_u64(bytes, definition_id.0);
            push_u64(bytes, producer_feature_id.0);
            push_string(bytes, semantic_role);
            push_string(bytes, source_element_id);
            write_feature_parameter_slot(bytes, *slot);
        }
    }
    push_u8(
        bytes,
        match dimension.presentation.unit {
            DimensionDisplayUnit::Millimetres => 1,
            DimensionDisplayUnit::Centimetres => 2,
            DimensionDisplayUnit::Inches => 3,
        },
    );
    push_u8(bytes, dimension.presentation.decimal_places);
}

fn write_feature_parameter_slot(bytes: &mut Vec<u8>, slot: FeatureParameterSlot) {
    push_u8(
        bytes,
        match slot {
            FeatureParameterSlot::Height => 1,
            FeatureParameterSlot::BodyRadius => 2,
            FeatureParameterSlot::BodyHeight => 3,
            FeatureParameterSlot::ShoulderRise => 4,
            FeatureParameterSlot::Thickness => 5,
            FeatureParameterSlot::Amount => 6,
            FeatureParameterSlot::ProfileWidth => 7,
            FeatureParameterSlot::ProfileHeight => 8,
        },
    );
}

fn write_feature_parameter_binding(bytes: &mut Vec<u8>, binding: &FeatureParameterBinding) {
    push_u64(bytes, binding.target.feature_id.0);
    push_u8(
        bytes,
        match binding.target.slot {
            FeatureParameterSlot::Height => 1,
            FeatureParameterSlot::BodyRadius => 2,
            FeatureParameterSlot::BodyHeight => 3,
            FeatureParameterSlot::ShoulderRise => 4,
            FeatureParameterSlot::Thickness => 5,
            FeatureParameterSlot::Amount => 6,
            FeatureParameterSlot::ProfileWidth => 7,
            FeatureParameterSlot::ProfileHeight => 8,
        },
    );
    write_identity(bytes, &binding.derived_from);
}

fn write_feature_parameter_provenance(
    bytes: &mut Vec<u8>,
    provenance: &FeatureParameterProvenance,
) {
    push_string(bytes, &provenance.identity.evaluator);
    push_string(bytes, &provenance.identity.schema);
    push_string(bytes, &provenance.identity.tolerance);
    if let Some(backend) = &provenance.identity.backend {
        push_u8(bytes, 1);
        push_string(bytes, backend);
    } else {
        push_u8(bytes, 0);
    }
    push_string(bytes, &provenance.input_digest);
    push_string(bytes, &provenance.result_digest);
    push_u64(bytes, provenance.applied_value_bits);
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
            FeatureKind::ThroughCut { target, profile } => {
                push_u8(bytes, 3);
                push_u64(bytes, target.0);
                push_u64(bytes, profile.0);
            }
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => {
                push_u8(bytes, 8);
                push_u8(
                    bytes,
                    match operation {
                        BooleanOperation::Cut => 1,
                        BooleanOperation::Union => 2,
                    },
                );
                push_u64(bytes, target.0);
                push_u64(bytes, tool.0);
            }
            FeatureKind::Revolve { profile } => {
                push_u8(bytes, 4);
                push_u64(bytes, profile.0);
            }
            FeatureKind::BottleProfileControl {
                profile,
                body_radius,
                body_height,
                shoulder_rise,
            } => {
                push_u8(bytes, 6);
                push_u64(bytes, profile.0);
                for dimension in [body_radius, body_height, shoulder_rise] {
                    push_string(bytes, dimension.source_token());
                    push_u64(bytes, dimension.millimetres().to_bits());
                }
            }
            FeatureKind::Shell { target, thickness } => {
                push_u8(bytes, 5);
                push_u64(bytes, target.0);
                push_string(bytes, thickness.source_token());
                push_u64(bytes, thickness.millimetres().to_bits());
            }
            FeatureKind::BottleEdgeFinish {
                target,
                kind,
                amount,
            } => {
                push_u8(bytes, 7);
                push_u64(bytes, target.0);
                push_u8(
                    bytes,
                    match kind {
                        BottleEdgeFinishKind::Fillet => 1,
                        BottleEdgeFinishKind::Chamfer => 2,
                    },
                );
                push_string(bytes, amount.source_token());
                push_u64(bytes, amount.millimetres().to_bits());
            }
            FeatureKind::MeshBody(spec) => {
                push_u8(bytes, 9);
                push_string(bytes, &spec.schema);
                push_u32(bytes, spec.vertices_mm.len() as u32);
                for vertex in &spec.vertices_mm {
                    for coordinate in vertex {
                        push_u64(bytes, coordinate.to_bits());
                    }
                }
                push_u32(bytes, spec.triangles.len() as u32);
                for triangle in &spec.triangles {
                    for index in triangle {
                        push_u32(bytes, *index);
                    }
                }
                match &spec.authority {
                    MeshAuthority::Authored { provenance } => {
                        push_u8(bytes, 1);
                        push_string(bytes, provenance);
                    }
                    MeshAuthority::ExactConversion(conversion) => {
                        push_u8(bytes, 2);
                        push_u64(bytes, conversion.source_document_id.0);
                        push_u64(bytes, conversion.source_revision);
                        push_string(bytes, &conversion.source_digest);
                        push_u64(bytes, conversion.source_definition_id.0);
                        push_u64(bytes, conversion.source_feature_id.0);
                        push_string(bytes, &conversion.source_result_fingerprint);
                        push_string(bytes, &conversion.source_evaluator);
                        push_string(bytes, &conversion.source_backend);
                        push_string(bytes, &conversion.source_tolerance);
                        push_string(bytes, &conversion.tessellation_tolerance);
                        push_u64(bytes, conversion.destination_definition_id.0);
                        push_u64(bytes, conversion.destination_feature_id.0);
                        push_u32(bytes, conversion.unsupported_semantics.len() as u32);
                        for semantic in &conversion.unsupported_semantics {
                            push_string(bytes, semantic);
                        }
                        push_u8(bytes, 1);
                    }
                }
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
    save_atomic_with_container(path, snapshot, &ContainerData::default())
}

pub fn save_atomic_with_container(
    path: impl AsRef<Path>,
    snapshot: &Snapshot,
    container_data: &ContainerData,
) -> Result<(), FilePersistenceError> {
    let path = path.as_ref();
    let bytes = save_container(snapshot, container_data).map_err(FilePersistenceError::Format)?;
    load(&bytes).map_err(FilePersistenceError::Format)?;
    if let Ok(previous) = fs::read(path)
        && load(&previous).is_ok()
    {
        write_atomic(&recovery_path(path), &previous)?;
    }
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), FilePersistenceError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| FilePersistenceError::Io(error.error))?;
    Ok(())
}

fn recovery_path(path: &Path) -> PathBuf {
    let mut recovery = path.as_os_str().to_os_string();
    recovery.push(".recovery");
    PathBuf::from(recovery)
}

pub fn load_file(path: impl AsRef<Path>) -> Result<LoadOutcome, FilePersistenceError> {
    let path = path.as_ref();
    match fs::read(path) {
        Ok(bytes) => match load(&bytes) {
            Ok(outcome) => Ok(outcome),
            Err(primary_error) => {
                try_load_recovery(path).ok_or(FilePersistenceError::Format(primary_error))
            }
        },
        Err(primary_error) => {
            try_load_recovery(path).ok_or(FilePersistenceError::Io(primary_error))
        }
    }
}

fn try_load_recovery(path: &Path) -> Option<LoadOutcome> {
    let bytes = fs::read(recovery_path(path)).ok()?;
    let mut outcome = load(&bytes).ok()?;
    match &mut outcome {
        LoadOutcome::Editable { audit, .. } => audit.recovered_from_backup = true,
        LoadOutcome::ReviewOnly(candidate) => candidate.audit.recovered_from_backup = true,
    }
    Some(outcome)
}

pub fn load(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    if bytes.starts_with(CONTAINER_MAGIC) {
        return load_container(bytes);
    }
    load_document(bytes, ContainerData::default())
}

fn load_container(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    if bytes.len() > MAX_CONTAINER_BYTES {
        return Err(PersistenceError::ResourceLimit);
    }
    let mut reader = Reader::new(bytes);
    if reader.take(CONTAINER_MAGIC.len())? != CONTAINER_MAGIC {
        return Err(PersistenceError::InvalidContainerMagic);
    }
    let schema = reader.u16()?;
    if schema != CONTAINER_SCHEMA {
        return Err(PersistenceError::UnsupportedContainerSchema(schema));
    }
    let entry_count = reader.count_with_limit(MAX_CONTAINER_ENTRIES)?;
    let mut entries = BTreeMap::<String, (bool, Vec<u8>)>::new();
    for _ in 0..entry_count {
        let path = reader.string()?;
        validate_container_path(&path)?;
        let required = reader.boolean()?;
        let length =
            usize::try_from(reader.u64()?).map_err(|_| PersistenceError::LengthOverflow)?;
        if length > MAX_SIDECAR_BYTES && path != "document.bin" {
            return Err(PersistenceError::ResourceLimit);
        }
        let checksum: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        let content = reader.take(length)?.to_vec();
        if crate::graph::sha256_bytes(&content) != checksum {
            return Err(PersistenceError::ContainerChecksumMismatch(path));
        }
        if entries.insert(path, (required, content)).is_some() {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
    }
    if !reader.is_finished() {
        return Err(PersistenceError::TrailingBytes);
    }

    let (document_required, document) = entries
        .remove("document.bin")
        .ok_or(PersistenceError::MissingDocumentEntry)?;
    if !document_required {
        return Err(PersistenceError::DocumentEntryNotRequired);
    }
    let mut container_data = ContainerData::default();
    for (path, (required, content)) in entries {
        if let Some(hash) = path.strip_prefix("blobs/") {
            if required || hash.len() != 64 || crate::graph::sha256_hex(&content) != hash {
                return Err(PersistenceError::InvalidBlobHash);
            }
            if container_data
                .blobs
                .insert(hash.to_owned(), content)
                .is_some()
            {
                return Err(PersistenceError::DuplicateContainerEntry);
            }
        } else if let Some(extension_path) = path.strip_prefix("extensions/") {
            let (namespace, relative_path) = extension_path
                .split_once('/')
                .ok_or_else(|| PersistenceError::InvalidContainerPath(path.clone()))?;
            container_data.insert_extension(ExtensionEntry::new(
                namespace,
                relative_path,
                required,
                content,
            )?)?;
        } else {
            return Err(PersistenceError::UnsupportedContainerEntry(path));
        }
    }
    load_document(&document, container_data)
}

fn load_document(
    bytes: &[u8],
    container_data: ContainerData,
) -> Result<LoadOutcome, PersistenceError> {
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
        LEGACY_SCHEMA
            | RESEARCH_SCHEMA
            | PRODUCT_SCHEMA
            | ENVELOPE_SCHEMA
            | EXACT_EVIDENCE_SCHEMA
            | THROUGH_CUT_SCHEMA
            | REVOLVE_SCHEMA
            | SHELL_SCHEMA
            | BOTTLE_FINISH_SCHEMA
            | BOOLEAN_SCHEMA
            | PARAMETRIC_BINDING_SCHEMA
            | PARAMETRIC_PROVENANCE_SCHEMA
            | PROFILE_CONSTRAINT_SCHEMA
            | PERSISTENT_DIMENSION_SCHEMA
            | TAG_SCHEMA
            | COLLECTION_SCHEMA
            | MESH_BODY_SCHEMA
            | CURRENT_SCHEMA
    ) {
        return Err(PersistenceError::UnsupportedSchema(schema));
    }
    let mut migration_losses = Vec::new();
    let (revision_id, product) = if schema >= ENVELOPE_SCHEMA {
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
        let product = read_product(
            &mut payload_reader,
            ProductSchemaCapabilities::current(schema),
        )?;
        if !payload_reader.is_finished() {
            return Err(PersistenceError::TrailingBytes);
        }
        (revision_id, product)
    } else {
        let revision_id = reader.u64()?;
        let nodes = read_nodes(&mut reader, schema == LEGACY_SCHEMA, &mut migration_losses)?;
        let mut product = if schema == PRODUCT_SCHEMA {
            read_product(&mut reader, ProductSchemaCapabilities::PRODUCT_V2)?
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
    let document = DocumentStore::from_product(revision_id, product)?;
    let feature_parameter_freshness = document
        .current()
        .audit_feature_parameter_freshness(&EvaluationIdentity::default())?;
    let audit = LoadAudit {
        source_schema: schema,
        migration_losses,
        override_health,
        feature_parameter_freshness,
        unknown_extensions: container_data
            .extensions()
            .map(|entry| ExtensionAudit {
                namespace: entry.namespace().to_owned(),
                path: entry.path().to_owned(),
                required: entry.required(),
            })
            .collect(),
        recovered_from_backup: false,
    };
    let loaded_snapshot = document.current();
    for reference in loaded_snapshot.exact_reference_evidence() {
        let matches_request = crate::exact_product::ExactFeatureChainRequest::from_snapshot(
            &loaded_snapshot,
            reference.definition_id,
        )
        .is_ok_and(|request| reference.matches_request(&request))
            || crate::bottle_m6::ExactRevolveRequest::from_snapshot(
                &loaded_snapshot,
                reference.definition_id,
            )
            .is_ok_and(|request| {
                crate::bottle_m6::reference_matches_revolve_request(reference, &request)
            });
        if !matches_request {
            return Err(PersistenceError::InvalidExactReference);
        }
    }
    if review_required || container_data.requires_unknown_extension() {
        Ok(LoadOutcome::ReviewOnly(ReviewCandidate {
            snapshot: document.current(),
            audit: Box::new(audit),
            container_data: Box::new(container_data),
        }))
    } else {
        Ok(LoadOutcome::Editable {
            document,
            audit,
            container_data,
        })
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

fn read_feature_parameter_slot(
    reader: &mut Reader<'_>,
) -> Result<FeatureParameterSlot, PersistenceError> {
    match reader.u8()? {
        1 => Ok(FeatureParameterSlot::Height),
        2 => Ok(FeatureParameterSlot::BodyRadius),
        3 => Ok(FeatureParameterSlot::BodyHeight),
        4 => Ok(FeatureParameterSlot::ShoulderRise),
        5 => Ok(FeatureParameterSlot::Thickness),
        6 => Ok(FeatureParameterSlot::Amount),
        7 => Ok(FeatureParameterSlot::ProfileWidth),
        8 => Ok(FeatureParameterSlot::ProfileHeight),
        value => Err(PersistenceError::InvalidParameterSlot(value)),
    }
}

fn read_persistent_dimension(
    reader: &mut Reader<'_>,
) -> Result<PersistentDimension, PersistenceError> {
    let id = PersistentDimensionId(reader.u64()?);
    let name = reader.string()?;
    let target = match reader.u8()? {
        1 => PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
            feature_id: FeatureId(reader.u64()?),
            slot: read_feature_parameter_slot(reader)?,
        }),
        2 => PersistentDimensionTarget::DerivedOutput(read_identity(reader)?),
        3 => PersistentDimensionTarget::ExactFeatureParameter {
            definition_id: DefinitionId(reader.u64()?),
            producer_feature_id: FeatureId(reader.u64()?),
            semantic_role: reader.string()?,
            source_element_id: reader.string()?,
            slot: read_feature_parameter_slot(reader)?,
        },
        value => return Err(PersistenceError::InvalidPersistentDimensionTarget(value)),
    };
    let unit = match reader.u8()? {
        1 => DimensionDisplayUnit::Millimetres,
        2 => DimensionDisplayUnit::Centimetres,
        3 => DimensionDisplayUnit::Inches,
        value => return Err(PersistenceError::InvalidDimensionDisplayUnit(value)),
    };
    let presentation = DimensionPresentation::new(unit, reader.u8()?)?;
    PersistentDimension::new(id, name, target, presentation).map_err(PersistenceError::from)
}

fn read_feature_parameter_binding(
    reader: &mut Reader<'_>,
) -> Result<FeatureParameterBinding, PersistenceError> {
    let feature_id = FeatureId(reader.u64()?);
    let slot = read_feature_parameter_slot(reader)?;
    Ok(FeatureParameterBinding {
        target: FeatureParameterTarget { feature_id, slot },
        derived_from: read_identity(reader)?,
    })
}

fn read_feature_parameter_provenance(
    reader: &mut Reader<'_>,
) -> Result<Option<FeatureParameterProvenance>, PersistenceError> {
    match reader.u8()? {
        0 => Ok(None),
        1 => Ok(Some(FeatureParameterProvenance {
            identity: EvaluationIdentity {
                evaluator: reader.string()?,
                schema: reader.string()?,
                tolerance: reader.string()?,
                backend: match reader.u8()? {
                    0 => None,
                    1 => Some(reader.string()?),
                    value => return Err(PersistenceError::InvalidOptionalMarker(value)),
                },
            },
            input_digest: reader.string()?,
            result_digest: reader.string()?,
            applied_value_bits: reader.u64()?,
        })),
        value => Err(PersistenceError::InvalidOptionalMarker(value)),
    }
}

fn read_joint(reader: &mut Reader<'_>) -> Result<CanonicalJoint, PersistenceError> {
    let id = JointId(reader.u64()?);
    let participant_a = read_identity(reader)?;
    let participant_b = read_identity(reader)?;
    let volume = read_bounded_volume(reader)?;
    CanonicalJoint::new(id, participant_a, participant_b, volume)
        .map_err(CanonicalError::from)
        .map_err(PersistenceError::from)
}

fn read_space(reader: &mut Reader<'_>) -> Result<CanonicalSpace, PersistenceError> {
    let id = SpaceId(reader.u64()?);
    let purpose = reader.string()?;
    let volume = read_bounded_volume(reader)?;
    let adjacent_to = read_ids(reader)?
        .into_iter()
        .map(SpaceId)
        .collect::<Vec<_>>();
    let accessible_to = read_ids(reader)?
        .into_iter()
        .map(SpaceId)
        .collect::<Vec<_>>();
    if adjacent_to.windows(2).any(|pair| pair[0] >= pair[1])
        || accessible_to.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(PersistenceError::InvalidCanonicalData(
            CanonicalError::Space(crate::space::SpaceError::InvalidRelation),
        ));
    }
    CanonicalSpace::new(id, purpose, volume, adjacent_to, accessible_to)
        .map_err(CanonicalError::from)
        .map_err(PersistenceError::from)
}

fn read_clearance_volume(
    reader: &mut Reader<'_>,
) -> Result<CanonicalClearanceVolume, PersistenceError> {
    let id = ClearanceVolumeId(reader.u64()?);
    let owner = match reader.u8()? {
        1 => {
            let mut path = InstancePath::root(OccurrenceId(reader.u64()?));
            for _ in 0..reader.count()? {
                path = path.with_step(match reader.u8()? {
                    1 => InstancePathStep::Group(LocalGroupId(reader.u64()?)),
                    2 => InstancePathStep::Occurrence(LocalOccurrenceId(reader.u64()?)),
                    value => return Err(PersistenceError::InvalidClearanceOwner(value)),
                });
            }
            ClearanceOwner::Occurrence(path)
        }
        2 => ClearanceOwner::Space(SpaceId(reader.u64()?)),
        value => return Err(PersistenceError::InvalidClearanceOwner(value)),
    };
    let reason = reader.string()?;
    let volume = read_bounded_volume(reader)?;
    if reader.u8()? != 1 {
        return Err(PersistenceError::InvalidClearanceCoordinateFrame);
    }
    let tolerance =
        TolerancePolicy::new(f64::from_bits(reader.u64()?)).map_err(CanonicalError::from)?;
    let severity = match reader.u8()? {
        1 => ClearanceSeverity::Advisory,
        2 => ClearanceSeverity::Required,
        value => return Err(PersistenceError::InvalidClearanceSeverity(value)),
    };
    let derived_from = match reader.u8()? {
        0 => None,
        1 => Some(read_identity(reader)?),
        value => return Err(PersistenceError::InvalidOptionalMarker(value)),
    };
    CanonicalClearanceVolume::new(id, owner, reason, volume, tolerance, severity, derived_from)
        .map_err(CanonicalError::from)
        .map_err(PersistenceError::from)
}

fn read_bounded_volume(reader: &mut Reader<'_>) -> Result<Aabb, PersistenceError> {
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
    Aabb::bounded_volume(min, max)
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
    capabilities: ProductSchemaCapabilities,
) -> Result<ProductModel, PersistenceError> {
    let mut product = ProductModel {
        document_id: crate::document::DocumentId(reader.u64()?),
        units: match reader.u8()? {
            1 => UnitSystem::Millimetres,
            units => return Err(PersistenceError::UnsupportedUnits(units)),
        },
        ..ProductModel::default()
    };
    if capabilities.current {
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
        if capabilities.parametric_bindings {
            for _ in 0..reader.count()? {
                let binding = read_feature_parameter_binding(reader)?;
                let target = binding.target;
                if product
                    .feature_parameter_bindings
                    .insert(target, Arc::new(binding))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateFeatureParameterBinding);
                }
                if capabilities.parametric_provenance
                    && let Some(provenance) = read_feature_parameter_provenance(reader)?
                {
                    product
                        .feature_parameter_provenance
                        .insert(target, Arc::new(provenance));
                }
            }
        }
    }
    for _ in 0..reader.count()? {
        let id = DefinitionId(reader.u64()?);
        let definition = Definition {
            id,
            name: reader.string()?,
            feature_ids: read_ids(reader)?.into_iter().map(FeatureId).collect(),
            local_group_ids: if capabilities.current {
                read_ids(reader)?.into_iter().map(LocalGroupId).collect()
            } else {
                Vec::new()
            },
            local_occurrence_ids: if capabilities.current {
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
            3 if capabilities.through_cut => FeatureKind::ThroughCut {
                target: FeatureId(reader.u64()?),
                profile: FeatureId(reader.u64()?),
            },
            4 if capabilities.revolve => FeatureKind::Revolve {
                profile: FeatureId(reader.u64()?),
            },
            5 if capabilities.shell => FeatureKind::Shell {
                target: FeatureId(reader.u64()?),
                thickness: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            6 if capabilities.bottle_finish => FeatureKind::BottleProfileControl {
                profile: FeatureId(reader.u64()?),
                body_radius: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                body_height: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                shoulder_rise: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            7 if capabilities.bottle_finish => FeatureKind::BottleEdgeFinish {
                target: FeatureId(reader.u64()?),
                kind: match reader.u8()? {
                    1 => BottleEdgeFinishKind::Fillet,
                    2 => BottleEdgeFinishKind::Chamfer,
                    value => return Err(PersistenceError::InvalidFeatureKind(value)),
                },
                amount: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            8 if capabilities.boolean => FeatureKind::Boolean {
                operation: match reader.u8()? {
                    1 => BooleanOperation::Cut,
                    2 => BooleanOperation::Union,
                    value => return Err(PersistenceError::InvalidFeatureKind(value)),
                },
                target: FeatureId(reader.u64()?),
                tool: FeatureId(reader.u64()?),
            },
            9 if capabilities.mesh_body => {
                let schema = reader.string()?;
                let mut vertices_mm = Vec::new();
                for _ in 0..reader.count()? {
                    vertices_mm.push([
                        f64::from_bits(reader.u64()?),
                        f64::from_bits(reader.u64()?),
                        f64::from_bits(reader.u64()?),
                    ]);
                }
                let mut triangles = Vec::new();
                for _ in 0..reader.count()? {
                    triangles.push([reader.u32()?, reader.u32()?, reader.u32()?]);
                }
                let authority = match reader.u8()? {
                    1 => MeshAuthority::Authored {
                        provenance: reader.string()?,
                    },
                    2 => {
                        let source_document_id = crate::document::DocumentId(reader.u64()?);
                        let source_revision = reader.u64()?;
                        let source_digest = reader.string()?;
                        let source_definition_id = DefinitionId(reader.u64()?);
                        let source_feature_id = FeatureId(reader.u64()?);
                        let source_result_fingerprint = reader.string()?;
                        let source_evaluator = reader.string()?;
                        let source_backend = reader.string()?;
                        let source_tolerance = reader.string()?;
                        let tessellation_tolerance = reader.string()?;
                        let destination_definition_id = DefinitionId(reader.u64()?);
                        let destination_feature_id = FeatureId(reader.u64()?);
                        let mut unsupported_semantics = Vec::new();
                        for _ in 0..reader.count()? {
                            unsupported_semantics.push(reader.string()?);
                        }
                        let exact_reference_consequence = match reader.u8()? {
                            1 => ExactReferenceConversionConsequence::Lost,
                            value => return Err(PersistenceError::InvalidFeatureKind(value)),
                        };
                        MeshAuthority::ExactConversion(ExactToMeshConversion {
                            source_document_id,
                            source_revision,
                            source_digest,
                            source_definition_id,
                            source_feature_id,
                            source_result_fingerprint,
                            source_evaluator,
                            source_backend,
                            source_tolerance,
                            tessellation_tolerance,
                            destination_definition_id,
                            destination_feature_id,
                            unsupported_semantics,
                            exact_reference_consequence,
                        })
                    }
                    value => return Err(PersistenceError::InvalidFeatureKind(value)),
                };
                FeatureKind::MeshBody(MeshBodySpec {
                    schema,
                    vertices_mm,
                    triangles,
                    authority,
                })
            }
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
    if capabilities.current {
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
        if capabilities.exact_evidence {
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
        if capabilities.persistent_dimensions {
            for _ in 0..reader.count()? {
                let dimension = read_persistent_dimension(reader)?;
                if product
                    .persistent_dimensions
                    .insert(dimension.id, Arc::new(dimension.clone()))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicatePersistentDimension(dimension.id));
                }
            }
        }
        if capabilities.tags {
            for _ in 0..reader.count()? {
                let id = TagId(reader.u64()?);
                let tag = Tag {
                    id,
                    name: reader.string()?,
                    visible: reader.boolean()?,
                };
                if product.tags.insert(id, Arc::new(tag)).is_some() {
                    return Err(PersistenceError::DuplicateTag(id));
                }
            }
        }
        if capabilities.collections {
            for _ in 0..reader.count()? {
                let id = CollectionId(reader.u64()?);
                let name = reader.string()?;
                let occurrence_ids = read_ids(reader)?
                    .into_iter()
                    .map(OccurrenceId)
                    .collect::<Vec<_>>();
                if occurrence_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::CollectionMembershipNotCanonical(id),
                    ));
                }
                let collection = Collection {
                    id,
                    name,
                    occurrence_ids: occurrence_ids.into_iter().collect::<BTreeSet<_>>(),
                };
                if product
                    .collections
                    .insert(id, Arc::new(collection))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateCollection(id));
                }
            }
        }
        if capabilities.space_clearance {
            for _ in 0..reader.count()? {
                let space = read_space(reader)?;
                if product.spaces.insert(space.id(), Arc::new(space)).is_some() {
                    return Err(PersistenceError::DuplicateSpace);
                }
            }
            for _ in 0..reader.count()? {
                let clearance = read_clearance_volume(reader)?;
                if product
                    .clearance_volumes
                    .insert(clearance.id(), Arc::new(clearance))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateClearanceVolume);
                }
            }
        }
    }
    Ok(product)
}

fn validate_namespace(namespace: &str) -> Result<(), PersistenceError> {
    if namespace.is_empty()
        || namespace.len() > MAX_CONTAINER_PATH_BYTES
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        return Err(PersistenceError::InvalidContainerPath(namespace.to_owned()));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), PersistenceError> {
    if path.is_empty()
        || path.len() > MAX_CONTAINER_PATH_BYTES
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('/')
        || path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(PersistenceError::InvalidContainerPath(path.to_owned()));
    }
    Ok(())
}

fn validate_container_path(path: &str) -> Result<(), PersistenceError> {
    validate_relative_path(path)?;
    if path.contains(':') {
        return Err(PersistenceError::InvalidContainerPath(path.to_owned()));
    }
    Ok(())
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
    InvalidContainerMagic,
    UnsupportedSchema(u16),
    UnsupportedContainerSchema(u16),
    MissingDocumentEntry,
    DocumentEntryNotRequired,
    DuplicateContainerEntry,
    InvalidContainerPath(String),
    UnsupportedContainerEntry(String),
    ContainerChecksumMismatch(String),
    InvalidBlobHash,
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
    InvalidOptionalMarker(u8),
    InvalidParameterSlot(u8),
    InvalidPersistentDimensionTarget(u8),
    InvalidDimensionDisplayUnit(u8),
    InvalidClearanceOwner(u8),
    InvalidClearanceCoordinateFrame,
    InvalidClearanceSeverity(u8),
    InvalidExactReference,
    ChecksumMismatch,
    ResourceLimit,
    UnsupportedEnvelopeIdentity,
    DuplicateOverride,
    DuplicateFeatureParameterBinding,
    DuplicateJoint,
    DuplicateSpace,
    DuplicateClearanceVolume,
    DuplicateExactReference,
    DuplicatePersistentDimension(PersistentDimensionId),
    DuplicateTag(TagId),
    DuplicateCollection(CollectionId),
    DuplicateNode(NodeId),
    DuplicateDefinition(DefinitionId),
    DuplicateFeature(FeatureId),
    DuplicateOccurrence(OccurrenceId),
    DuplicateGroup(GroupId),
    DuplicateLocalGroup(LocalGroupKey),
    DuplicateLocalOccurrence(LocalOccurrenceKey),
    MigrationNotConfirmable(&'static str),
    InvalidCanonicalData(CanonicalError),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("document is truncated"),
            Self::InvalidMagic => formatter.write_str("document magic is invalid"),
            Self::InvalidContainerMagic => formatter.write_str("container magic is invalid"),
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "document schema {schema} is unsupported")
            }
            Self::UnsupportedContainerSchema(schema) => {
                write!(formatter, "container schema {schema} is unsupported")
            }
            Self::MissingDocumentEntry => formatter.write_str("container has no document.bin"),
            Self::DocumentEntryNotRequired => {
                formatter.write_str("container document.bin must be required")
            }
            Self::DuplicateContainerEntry => formatter.write_str("container repeats an entry"),
            Self::InvalidContainerPath(path) => {
                write!(formatter, "container path {path:?} is unsafe")
            }
            Self::UnsupportedContainerEntry(path) => {
                write!(formatter, "container entry {path:?} is unsupported")
            }
            Self::ContainerChecksumMismatch(path) => {
                write!(
                    formatter,
                    "container entry {path:?} checksum does not match"
                )
            }
            Self::InvalidBlobHash => {
                formatter.write_str("container blob content hash does not match its path")
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
            Self::InvalidOptionalMarker(value) => {
                write!(formatter, "optional value marker {value} is invalid")
            }
            Self::InvalidParameterSlot(value) => {
                write!(formatter, "feature parameter slot {value} is invalid")
            }
            Self::InvalidPersistentDimensionTarget(value) => {
                write!(formatter, "persistent dimension target {value} is invalid")
            }
            Self::InvalidDimensionDisplayUnit(value) => {
                write!(formatter, "dimension display unit {value} is invalid")
            }
            Self::InvalidClearanceOwner(value) => {
                write!(formatter, "clearance owner kind {value} is invalid")
            }
            Self::InvalidClearanceCoordinateFrame => {
                formatter.write_str("clearance coordinate frame is invalid")
            }
            Self::InvalidClearanceSeverity(value) => {
                write!(formatter, "clearance severity {value} is invalid")
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
            Self::DuplicateFeatureParameterBinding => {
                formatter.write_str("document repeats a feature parameter binding")
            }
            Self::DuplicateJoint => formatter.write_str("document repeats a joint"),
            Self::DuplicateSpace => formatter.write_str("document repeats a space"),
            Self::DuplicateClearanceVolume => {
                formatter.write_str("document repeats a clearance volume")
            }
            Self::DuplicateExactReference => {
                formatter.write_str("document repeats exact reference evidence")
            }
            Self::DuplicatePersistentDimension(id) => {
                write!(formatter, "document repeats persistent dimension {}", id.0)
            }
            Self::DuplicateTag(id) => write!(formatter, "document repeats tag {}", id.0),
            Self::DuplicateCollection(id) => {
                write!(formatter, "document repeats collection {}", id.0)
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
            Self::MigrationNotConfirmable(reason) => {
                write!(
                    formatter,
                    "semantic migration cannot be confirmed: {reason}"
                )
            }
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
