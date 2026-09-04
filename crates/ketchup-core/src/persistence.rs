use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::assembly::{
    ASSEMBLY_MATE_SCHEMA_V1, AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
    AssemblyReferenceHealth,
};
use crate::assembly_joint::{
    ASSEMBLY_JOINT_SCHEMA_V1, ASSEMBLY_MOTION_STUDY_SCHEMA_V1, AssemblyJoint, AssemblyJointAxis,
    AssemblyJointId, AssemblyJointKind, AssemblyJointLimits, AssemblyMotionDriver,
    AssemblyMotionStudy, AssemblyMotionStudyId,
};
use crate::document::{
    BOTTLE_SHELL_OPENING_FACE_ROLE, BOTTLE_SHOULDER_EDGE_ROLE, Body, BodyId, BooleanOperation,
    BottleEdgeFinishKind, CanonicalCommand, CanonicalError, ClassificationCategory,
    ClassificationCategoryId, ClassificationDimension, ClassificationDimensionId, Collection,
    CollectionId, CommandBatch, Definition, DefinitionId, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DocumentStore, EdgeFinishKind, EvaluationIdentity, EvaluatorNode,
    ExactReferenceConversionConsequence, ExactToMeshConversion, Feature, FeatureBodyOwnership,
    FeatureId, FeatureKind, FeatureParameterBinding, FeatureParameterFreshnessAudit,
    FeatureParameterProvenance, FeatureParameterTarget, Group, GroupId, ImportedExactBodySpec,
    InstancePath, InstancePathStep, LocalGroup, LocalGroupId, LocalGroupKey, LocalOccurrence,
    LocalOccurrenceId, LocalOccurrenceKey, LoftSection, MeshAuthority, MeshBodySpec, NodeId,
    Occurrence, OccurrenceId, ParameterPath, ParameterValueType, PersistentDimension,
    PersistentDimensionId, PersistentDimensionTarget, ProductModel, ProfileSegment, Snapshot,
    SpatialPathSegment, StableEdgeRole, StableFaceRole, Tag, TagId, Transform, UnitSystem,
};
use crate::drawing::{DrawingSheet, DrawingSheetId, DrawingSource, ORTHOGRAPHIC_DRAWING_SCHEMA_V1};
use crate::exact_product::{BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ReferenceStability};
use crate::graph::{
    CanonicalOverride, DerivedIdentity, EvaluatorNodeKind, OverrideMergePolicy,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment,
};
use crate::import::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
    MAX_IMPORT_DIAGNOSTICS, MAX_IMPORT_OUTPUTS,
};
use crate::mechanical_contract::{
    MECHANICAL_CONDITION_SCHEMA_V1, MECHANICAL_INTERFACE_SCHEMA_V1, MechanicalAxisAlignment,
    MechanicalCondition, MechanicalConditionId, MechanicalConditionKind, MechanicalInterface,
    MechanicalInterfaceId, MechanicalPlanarFrame, MechanicalRole,
};
use crate::mechanical_coupling::{
    ASSEMBLY_MOTION_COUPLING_SCHEMA_V1, AssemblyMotionCoupling, AssemblyMotionCouplingId,
    AssemblyMotionDirection, AssemblyTransmissionKind, GearMeshKind, ScrewHandedness,
};
use crate::prismatic::{Aabb, CanonicalJoint, JointId, TolerancePolicy};
use crate::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, MAX_SKETCH_CONSTRAINTS, MAX_SKETCH_ENTITIES,
    PadSpec, PocketSpec, PrincipalPlane, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind, SketchPointRef,
    SketchRegionId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use crate::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceCoordinateFrame, ClearanceOwner,
    ClearanceSeverity, ClearanceVolumeId, SpaceId,
};
use crate::topology::TopologicalElementRef;

const MAGIC: &[u8; 10] = b"KETCHUPDOC";
const CONTAINER_MAGIC: &[u8; 10] = b"KETCHUPCTR";
const CONTAINER_SCHEMA: u16 = 1;
const MESH_BODY_SCHEMA: u16 = 16;
const SPACE_CLEARANCE_SCHEMA: u16 = 17;
const POCKET_SCHEMA: u16 = 18;
const SEGMENT_PROFILE_SCHEMA: u16 = 19;
const GENERAL_REVOLVE_SCHEMA: u16 = 20;
const STABLE_SUBSHAPE_ROLE_SCHEMA: u16 = 21;
const BOOLEAN_INTERSECT_SCHEMA: u16 = 22;
const BOOLEAN_SPLIT_SCHEMA: u16 = 23;
const PLANAR_OFFSET_SCHEMA: u16 = 24;
const SWEEP_SCHEMA: u16 = 25;
const LOFT_SPLINE_SCHEMA: u16 = 26;
const IMPORT_RECEIPT_SCHEMA: u16 = 27;
const IMPORTED_EXACT_BODY_SCHEMA: u16 = 29;
const SKETCHUP_SCENE_SCHEMA: u16 = 30;
const WORKPLANE_SKETCH_SCHEMA: u16 = 31;
const ASSEMBLY_CONTRACT_SCHEMA: u16 = 32;
const ORTHOGRAPHIC_DRAWING_SCHEMA: u16 = 33;
const BODY_CONTRACT_SCHEMA: u16 = 34;
const BODY_CONSUMPTION_SCHEMA: u16 = 35;
const BODY_FEATURE_SUPPRESSION_SCHEMA: u16 = 36;
const CLASSIFICATION_DIMENSION_SCHEMA: u16 = 37;
const FEATURE_EXTENT_SCHEMA: u16 = 38;
const IMPORTED_TOPOLOGY_COUNTS_SCHEMA: u16 = 39;
const TOPOLOGICAL_FEATURE_REFERENCE_SCHEMA: u16 = 40;
const GENERAL_PARAMETER_PATH_SCHEMA: u16 = 41;
const ASSEMBLY_KINEMATICS_SCHEMA: u16 = 42;
const ASSEMBLY_MOTION_COUPLING_SCHEMA: u16 = 43;
const MECHANICAL_CONTRACT_SCHEMA: u16 = 44;
const SKETCH_CONSTRAINT_VOCABULARY_SCHEMA: u16 = 45;
const RIGID_TRANSFORM_FEATURE_SCHEMA: u16 = 46;
const CUBIC_BEZIER_SKETCH_SCHEMA: u16 = 47;
const CUBIC_BEZIER_SEGMENT_PROFILE_SCHEMA: u16 = 48;
const SPATIAL_SWEEP_PATH_SCHEMA: u16 = 49;
pub const CURRENT_SCHEMA: u16 = SPATIAL_SWEEP_PATH_SCHEMA;
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
    pocket: bool,
    segment_profile: bool,
    general_revolve: bool,
    stable_subshape_roles: bool,
    boolean_intersect: bool,
    boolean_split: bool,
    planar_offset: bool,
    sweep: bool,
    loft_spline: bool,
    import_receipts: bool,
    imported_exact_body: bool,
    imported_topology_counts: bool,
    topological_feature_references: bool,
    general_parameter_paths: bool,
    sketchup_scene: bool,
    workplane_sketch: bool,
    assembly_contract: bool,
    orthographic_drawing: bool,
    body_contract: bool,
    body_consumption: bool,
    body_feature_suppression: bool,
    classification_dimensions: bool,
    feature_extents: bool,
    assembly_kinematics: bool,
    assembly_motion_couplings: bool,
    mechanical_contract: bool,
    sketch_constraint_vocabulary: bool,
    rigid_transform_feature: bool,
    cubic_bezier_sketch: bool,
    cubic_bezier_segment_profile: bool,
    spatial_sweep_path: bool,
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
        pocket: false,
        segment_profile: false,
        general_revolve: false,
        stable_subshape_roles: false,
        boolean_intersect: false,
        boolean_split: false,
        planar_offset: false,
        sweep: false,
        loft_spline: false,
        import_receipts: false,
        imported_exact_body: false,
        imported_topology_counts: false,
        topological_feature_references: false,
        general_parameter_paths: false,
        sketchup_scene: false,
        workplane_sketch: false,
        assembly_contract: false,
        orthographic_drawing: false,
        body_contract: false,
        body_consumption: false,
        body_feature_suppression: false,
        classification_dimensions: false,
        feature_extents: false,
        assembly_kinematics: false,
        assembly_motion_couplings: false,
        mechanical_contract: false,
        sketch_constraint_vocabulary: false,
        rigid_transform_feature: false,
        cubic_bezier_sketch: false,
        cubic_bezier_segment_profile: false,
        spatial_sweep_path: false,
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
            pocket: schema >= POCKET_SCHEMA,
            segment_profile: schema >= SEGMENT_PROFILE_SCHEMA,
            general_revolve: schema >= GENERAL_REVOLVE_SCHEMA,
            stable_subshape_roles: schema >= STABLE_SUBSHAPE_ROLE_SCHEMA,
            boolean_intersect: schema >= BOOLEAN_INTERSECT_SCHEMA,
            boolean_split: schema >= BOOLEAN_SPLIT_SCHEMA,
            planar_offset: schema >= PLANAR_OFFSET_SCHEMA,
            sweep: schema >= SWEEP_SCHEMA,
            loft_spline: schema >= LOFT_SPLINE_SCHEMA,
            import_receipts: schema >= IMPORT_RECEIPT_SCHEMA,
            imported_exact_body: schema >= IMPORTED_EXACT_BODY_SCHEMA,
            imported_topology_counts: schema >= IMPORTED_TOPOLOGY_COUNTS_SCHEMA,
            topological_feature_references: schema >= TOPOLOGICAL_FEATURE_REFERENCE_SCHEMA,
            general_parameter_paths: schema >= GENERAL_PARAMETER_PATH_SCHEMA,
            sketchup_scene: schema >= SKETCHUP_SCENE_SCHEMA,
            workplane_sketch: schema >= WORKPLANE_SKETCH_SCHEMA,
            assembly_contract: schema >= ASSEMBLY_CONTRACT_SCHEMA,
            orthographic_drawing: schema >= ORTHOGRAPHIC_DRAWING_SCHEMA,
            body_contract: schema >= BODY_CONTRACT_SCHEMA,
            body_consumption: schema >= BODY_CONSUMPTION_SCHEMA,
            body_feature_suppression: schema >= BODY_FEATURE_SUPPRESSION_SCHEMA,
            classification_dimensions: schema >= CLASSIFICATION_DIMENSION_SCHEMA,
            feature_extents: schema >= FEATURE_EXTENT_SCHEMA,
            assembly_kinematics: schema >= ASSEMBLY_KINEMATICS_SCHEMA,
            assembly_motion_couplings: schema >= ASSEMBLY_MOTION_COUPLING_SCHEMA,
            mechanical_contract: schema >= MECHANICAL_CONTRACT_SCHEMA,
            sketch_constraint_vocabulary: schema >= SKETCH_CONSTRAINT_VOCABULARY_SCHEMA,
            rigid_transform_feature: schema >= RIGID_TRANSFORM_FEATURE_SCHEMA,
            cubic_bezier_sketch: schema >= CUBIC_BEZIER_SKETCH_SCHEMA,
            cubic_bezier_segment_profile: schema >= CUBIC_BEZIER_SEGMENT_PROFILE_SCHEMA,
            spatial_sweep_path: schema >= SPATIAL_SWEEP_PATH_SCHEMA,
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
    imported_source_blobs: Arc<BTreeSet<String>>,
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

    pub fn insert_import_blob(&mut self, bytes: Vec<u8>) -> Result<String, PersistenceError> {
        let hash = self.insert_blob(bytes)?;
        Arc::make_mut(&mut self.imported_source_blobs).insert(hash.clone());
        Ok(hash)
    }

    pub fn insert_extension(&mut self, entry: ExtensionEntry) -> Result<(), PersistenceError> {
        let key = (entry.namespace.clone(), entry.path.clone());
        if self.extensions.insert(key, entry).is_some() {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
        Ok(())
    }

    pub fn set_extension(&mut self, entry: ExtensionEntry) {
        let key = (entry.namespace.clone(), entry.path.clone());
        self.extensions.insert(key, entry);
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
    push_u32(&mut payload, product.import_receipts.len() as u32);
    for receipt in product.import_receipts.values() {
        write_import_receipt(&mut payload, receipt);
    }
    push_u32(&mut payload, product.spaces.len() as u32);
    for space in product.spaces.values() {
        write_space(&mut payload, space);
    }
    push_u32(&mut payload, product.clearance_volumes.len() as u32);
    for clearance in product.clearance_volumes.values() {
        write_clearance_volume(&mut payload, clearance);
    }
    push_u32(&mut payload, product.grounded_occurrences.len() as u32);
    for occurrence_id in &product.grounded_occurrences {
        push_u64(&mut payload, occurrence_id.0);
    }
    push_u32(&mut payload, product.assembly_mates.len() as u32);
    for mate in product.assembly_mates.values() {
        write_assembly_mate(&mut payload, mate);
    }
    push_u32(&mut payload, product.drawing_sheets.len() as u32);
    for sheet in product.drawing_sheets.values() {
        write_drawing_sheet(&mut payload, sheet);
    }
    push_u32(&mut payload, product.definitions.len() as u32);
    for definition in product.definitions.values() {
        push_u64(&mut payload, definition.id().0);
        push_u32(&mut payload, definition.bodies.len() as u32);
        for body in definition.bodies.values() {
            push_u64(&mut payload, body.id().0);
            push_string(&mut payload, body.name());
            push_u8(&mut payload, u8::from(body.visible()));
            push_optional_id(&mut payload, body.consumed_by().map(|id| id.0));
        }
        push_u64(&mut payload, definition.active_body_id().0);
        push_u32(&mut payload, definition.feature_body_ownership.len() as u32);
        for (feature_id, ownership) in &definition.feature_body_ownership {
            push_u64(&mut payload, feature_id.0);
            write_ids(
                &mut payload,
                ownership.input_body_ids().iter().map(|id| id.0),
            );
            push_optional_id(&mut payload, ownership.output_body_id().map(|id| id.0));
        }
    }
    push_u32(&mut payload, product.body_feature_suppression.len() as u32);
    for ((definition_id, body_id), suppressed) in &product.body_feature_suppression {
        push_u64(&mut payload, definition_id.0);
        push_u64(&mut payload, body_id.0);
        write_ids(&mut payload, suppressed.iter().map(|id| id.0));
    }
    push_u32(&mut payload, product.classification_dimensions.len() as u32);
    for dimension in product.classification_dimensions.values() {
        push_u64(&mut payload, dimension.id().0);
        push_string(&mut payload, dimension.name());
        push_u32(&mut payload, dimension.categories().count() as u32);
        for category in dimension.categories() {
            push_u64(&mut payload, category.id().0);
            push_string(&mut payload, category.name());
        }
    }
    push_u32(
        &mut payload,
        product.classification_assignments.len() as u32,
    );
    for ((occurrence_id, dimension_id), category_id) in &product.classification_assignments {
        push_u64(&mut payload, occurrence_id.0);
        push_u64(&mut payload, dimension_id.0);
        push_u64(&mut payload, category_id.0);
    }
    push_u32(&mut payload, product.assembly_joints.len() as u32);
    for joint in product.assembly_joints.values() {
        write_assembly_joint(&mut payload, joint);
    }
    push_u32(&mut payload, product.assembly_motion_couplings.len() as u32);
    for coupling in product.assembly_motion_couplings.values() {
        write_assembly_motion_coupling(&mut payload, coupling);
    }
    push_u32(&mut payload, product.assembly_motion_studies.len() as u32);
    for study in product.assembly_motion_studies.values() {
        write_assembly_motion_study(&mut payload, study);
    }
    push_u32(&mut payload, product.mechanical_interfaces.len() as u32);
    for interface in product.mechanical_interfaces.values() {
        write_mechanical_interface(&mut payload, interface);
    }
    push_u32(&mut payload, product.mechanical_conditions.len() as u32);
    for condition in product.mechanical_conditions.values() {
        write_mechanical_condition(&mut payload, condition);
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

fn imported_source_blob_hashes(snapshot: &Snapshot) -> BTreeSet<String> {
    snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(
                spec.source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

pub fn save_container(
    snapshot: &Snapshot,
    container_data: &ContainerData,
) -> Result<Vec<u8>, PersistenceError> {
    let mut entries = BTreeMap::<String, (bool, Vec<u8>)>::new();
    entries.insert("document.bin".to_owned(), (true, save(snapshot)));
    let imported_sources = imported_source_blob_hashes(snapshot);
    if imported_sources
        .iter()
        .any(|hash| !container_data.blobs.contains_key(hash))
    {
        return Err(PersistenceError::InvalidBlobHash);
    }
    for (hash, bytes) in &container_data.blobs {
        if crate::graph::sha256_hex(bytes) != *hash {
            return Err(PersistenceError::InvalidBlobHash);
        }
        if !container_data.imported_source_blobs.contains(hash) || imported_sources.contains(hash) {
            entries.insert(format!("blobs/{hash}"), (false, bytes.clone()));
        }
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

fn write_feature_direction(bytes: &mut Vec<u8>, direction: FeatureDirection) {
    match direction {
        FeatureDirection::AlongNormal => push_u8(bytes, 1),
        FeatureDirection::OppositeNormal => push_u8(bytes, 2),
        FeatureDirection::Vector(vector) => {
            push_u8(bytes, 3);
            for component in vector {
                push_u64(bytes, component.to_bits());
            }
        }
    }
}

fn write_extent_dimension(bytes: &mut Vec<u8>, distance: &Dimension) {
    push_string(bytes, distance.source_token());
    push_u64(bytes, distance.millimetres().to_bits());
}

fn write_feature_extent_end(bytes: &mut Vec<u8>, end: &FeatureExtentEnd) {
    match end {
        FeatureExtentEnd::Blind(distance) => {
            push_u8(bytes, 1);
            write_extent_dimension(bytes, distance);
        }
        FeatureExtentEnd::ThroughAll => push_u8(bytes, 2),
        FeatureExtentEnd::UpToFace(reference) => {
            push_u8(bytes, 3);
            write_exact_reference(bytes, reference);
        }
    }
}

fn write_feature_extent(bytes: &mut Vec<u8>, extent: &FeatureExtent) {
    match extent {
        FeatureExtent::Blind(distance) => {
            push_u8(bytes, 1);
            write_extent_dimension(bytes, distance);
        }
        FeatureExtent::ThroughAll => push_u8(bytes, 2),
        FeatureExtent::UpToFace(reference) => {
            push_u8(bytes, 3);
            write_exact_reference(bytes, reference);
        }
        FeatureExtent::Symmetric(distance) => {
            push_u8(bytes, 4);
            write_extent_dimension(bytes, distance);
        }
        FeatureExtent::Bidirectional { along, opposite } => {
            push_u8(bytes, 5);
            write_feature_extent_end(bytes, along);
            write_feature_extent_end(bytes, opposite);
        }
    }
}

fn write_persistent_dimension(bytes: &mut Vec<u8>, dimension: &PersistentDimension) {
    push_u64(bytes, dimension.id.0);
    push_string(bytes, &dimension.name);
    match &dimension.target {
        PersistentDimensionTarget::FeatureParameter(target) => {
            push_u8(bytes, 1);
            write_feature_parameter_target(bytes, target);
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
            path,
            value_type,
        } => {
            push_u8(bytes, 3);
            push_u64(bytes, definition_id.0);
            write_feature_parameter_target(
                bytes,
                &FeatureParameterTarget {
                    feature_id: *producer_feature_id,
                    path: path.clone(),
                    value_type: *value_type,
                },
            );
            push_string(bytes, semantic_role);
            push_string(bytes, source_element_id);
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

fn write_feature_parameter_target(bytes: &mut Vec<u8>, target: &FeatureParameterTarget) {
    push_u64(bytes, target.feature_id.0);
    push_string(bytes, target.path.as_str());
    push_u8(
        bytes,
        match target.value_type {
            ParameterValueType::Length => 1,
            ParameterValueType::Angle => 2,
            ParameterValueType::Scalar => 3,
        },
    );
}

fn write_feature_parameter_binding(bytes: &mut Vec<u8>, binding: &FeatureParameterBinding) {
    write_feature_parameter_target(bytes, &binding.target);
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

fn write_workplane(bytes: &mut Vec<u8>, spec: &WorkplaneSpec) {
    match &spec.support {
        WorkplaneSupport::Principal(plane) => {
            push_u8(bytes, 1);
            push_u8(
                bytes,
                match plane {
                    PrincipalPlane::Xy => 1,
                    PrincipalPlane::Yz => 2,
                    PrincipalPlane::Xz => 3,
                },
            );
        }
        WorkplaneSupport::Offset { base, distance } => {
            push_u8(bytes, 2);
            push_u64(bytes, base.0);
            push_string(bytes, distance.source_token());
            push_u64(bytes, distance.millimetres().to_bits());
        }
        WorkplaneSupport::PlanarFace { reference, health } => {
            push_u8(bytes, 3);
            write_exact_reference(bytes, reference);
            push_u8(
                bytes,
                match health {
                    WorkplaneSupportHealth::Resolved => 1,
                    WorkplaneSupportHealth::Ambiguous => 2,
                    WorkplaneSupportHealth::Lost => 3,
                    WorkplaneSupportHealth::Stale => 4,
                },
            );
        }
    }
    for coordinate in spec
        .frame
        .origin_mm
        .iter()
        .chain(spec.frame.x_axis.iter())
        .chain(spec.frame.y_axis.iter())
        .chain(spec.frame.normal.iter())
    {
        push_u64(bytes, coordinate.to_bits());
    }
}

fn write_sketch_point_ref(bytes: &mut Vec<u8>, reference: SketchPointRef) {
    push_u64(bytes, reference.entity.0);
    push_u8(
        bytes,
        match reference.point {
            SketchPointKind::Start => 1,
            SketchPointKind::End => 2,
            SketchPointKind::Center => 3,
            SketchPointKind::Control1 => 4,
            SketchPointKind::Control2 => 5,
        },
    );
}

fn write_sketch(bytes: &mut Vec<u8>, spec: &SketchSpec) {
    push_u64(bytes, spec.workplane.0);
    push_u32(bytes, spec.entities.len() as u32);
    for entity in &spec.entities {
        match entity {
            SketchEntity::Line {
                id,
                start_mm,
                end_mm,
            } => {
                push_u8(bytes, 1);
                push_u64(bytes, id.0);
                for point in [start_mm, end_mm] {
                    push_u64(bytes, point[0].to_bits());
                    push_u64(bytes, point[1].to_bits());
                }
            }
            SketchEntity::Arc {
                id,
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                push_u8(bytes, 2);
                push_u64(bytes, id.0);
                for point in [start_mm, end_mm, center_mm] {
                    push_u64(bytes, point[0].to_bits());
                    push_u64(bytes, point[1].to_bits());
                }
                push_u8(bytes, u8::from(*clockwise));
            }
            SketchEntity::Circle {
                id,
                center_mm,
                radius_mm,
            } => {
                push_u8(bytes, 3);
                push_u64(bytes, id.0);
                push_u64(bytes, center_mm[0].to_bits());
                push_u64(bytes, center_mm[1].to_bits());
                push_u64(bytes, radius_mm.to_bits());
            }
            SketchEntity::CubicBezier {
                id,
                start_mm,
                control_1_mm,
                control_2_mm,
                end_mm,
            } => {
                push_u8(bytes, 4);
                push_u64(bytes, id.0);
                for point in [start_mm, control_1_mm, control_2_mm, end_mm] {
                    push_u64(bytes, point[0].to_bits());
                    push_u64(bytes, point[1].to_bits());
                }
            }
        }
    }
    push_u32(bytes, spec.constraints.len() as u32);
    for constraint in &spec.constraints {
        push_u64(bytes, constraint.id.0);
        match &constraint.kind {
            SketchConstraintKind::Horizontal { entity } => {
                push_u8(bytes, 1);
                push_u64(bytes, entity.0);
            }
            SketchConstraintKind::Vertical { entity } => {
                push_u8(bytes, 2);
                push_u64(bytes, entity.0);
            }
            SketchConstraintKind::Coincident { a, b } => {
                push_u8(bytes, 3);
                write_sketch_point_ref(bytes, *a);
                write_sketch_point_ref(bytes, *b);
            }
            SketchConstraintKind::Distance { a, b, value } => {
                push_u8(bytes, 4);
                write_sketch_point_ref(bytes, *a);
                write_sketch_point_ref(bytes, *b);
                push_string(bytes, value.source_token());
                push_u64(bytes, value.millimetres().to_bits());
            }
            SketchConstraintKind::Radius { entity, value } => {
                push_u8(bytes, 5);
                push_u64(bytes, entity.0);
                push_string(bytes, value.source_token());
                push_u64(bytes, value.millimetres().to_bits());
            }
            SketchConstraintKind::FixedPoint { point, position_mm } => {
                push_u8(bytes, 6);
                write_sketch_point_ref(bytes, *point);
                push_u64(bytes, position_mm[0].to_bits());
                push_u64(bytes, position_mm[1].to_bits());
            }
            SketchConstraintKind::Parallel { a, b } => {
                push_u8(bytes, 7);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
            }
            SketchConstraintKind::Perpendicular { a, b } => {
                push_u8(bytes, 8);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
            }
            SketchConstraintKind::Tangent { a, b } => {
                push_u8(bytes, 9);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
            }
            SketchConstraintKind::Angle {
                a,
                b,
                angle_degrees,
            } => {
                push_u8(bytes, 10);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
                push_u64(bytes, angle_degrees.to_bits());
            }
            SketchConstraintKind::Equal { a, b } => {
                push_u8(bytes, 11);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
            }
            SketchConstraintKind::Symmetric { a, b, axis } => {
                push_u8(bytes, 12);
                write_sketch_point_ref(bytes, *a);
                write_sketch_point_ref(bytes, *b);
                push_u64(bytes, axis.0);
            }
            SketchConstraintKind::Concentric { a, b } => {
                push_u8(bytes, 13);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
            }
            SketchConstraintKind::Collinear { a, b } => {
                push_u8(bytes, 14);
                push_u64(bytes, a.0);
                push_u64(bytes, b.0);
            }
            SketchConstraintKind::Midpoint { point, line } => {
                push_u8(bytes, 15);
                write_sketch_point_ref(bytes, *point);
                push_u64(bytes, line.0);
            }
            SketchConstraintKind::PointOnCurve { point, curve } => {
                push_u8(bytes, 16);
                write_sketch_point_ref(bytes, *point);
                push_u64(bytes, curve.0);
            }
        }
    }
}

fn write_features(bytes: &mut Vec<u8>, product: &ProductModel) {
    push_u32(bytes, product.features.len() as u32);
    for feature in product.features.values() {
        push_u64(bytes, feature.id().0);
        push_u64(bytes, feature.definition_id().0);
        push_string(bytes, feature.name());
        match feature.kind() {
            FeatureKind::Workplane(spec) => {
                push_u8(bytes, 17);
                write_workplane(bytes, spec);
            }
            FeatureKind::Sketch(spec) => {
                push_u8(bytes, 18);
                write_sketch(bytes, spec);
            }
            FeatureKind::Profile { points_mm } => {
                push_u8(bytes, 1);
                push_u32(bytes, points_mm.len() as u32);
                for point in points_mm {
                    push_u64(bytes, point[0].to_bits());
                    push_u64(bytes, point[1].to_bits());
                }
            }
            FeatureKind::SegmentProfile { segments, closed } => {
                push_u8(bytes, 11);
                push_u8(bytes, u8::from(*closed));
                push_u32(bytes, segments.len() as u32);
                for segment in segments {
                    match segment {
                        ProfileSegment::Line { start_mm, end_mm } => {
                            push_u8(bytes, 1);
                            for point in [start_mm, end_mm] {
                                push_u64(bytes, point[0].to_bits());
                                push_u64(bytes, point[1].to_bits());
                            }
                        }
                        ProfileSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => {
                            push_u8(bytes, 2);
                            for point in [start_mm, end_mm, center_mm] {
                                push_u64(bytes, point[0].to_bits());
                                push_u64(bytes, point[1].to_bits());
                            }
                            push_u8(bytes, u8::from(*clockwise));
                        }
                        ProfileSegment::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => {
                            push_u8(bytes, 3);
                            for point in [start_mm, control_1_mm, control_2_mm, end_mm] {
                                push_u64(bytes, point[0].to_bits());
                                push_u64(bytes, point[1].to_bits());
                            }
                        }
                    }
                }
            }
            FeatureKind::SpatialPath { segments } => {
                push_u8(bytes, 25);
                push_u32(bytes, segments.len() as u32);
                for segment in segments {
                    match segment {
                        SpatialPathSegment::Line { start_mm, end_mm } => {
                            push_u8(bytes, 1);
                            for point in [start_mm, end_mm] {
                                for coordinate in point {
                                    push_u64(bytes, coordinate.to_bits());
                                }
                            }
                        }
                        SpatialPathSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            normal,
                            clockwise,
                        } => {
                            push_u8(bytes, 2);
                            for point in [start_mm, end_mm, center_mm, normal] {
                                for coordinate in point {
                                    push_u64(bytes, coordinate.to_bits());
                                }
                            }
                            push_u8(bytes, u8::from(*clockwise));
                        }
                        SpatialPathSegment::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => {
                            push_u8(bytes, 3);
                            for point in [start_mm, control_1_mm, control_2_mm, end_mm] {
                                for coordinate in point {
                                    push_u64(bytes, coordinate.to_bits());
                                }
                            }
                        }
                    }
                }
            }
            FeatureKind::SplineProfile { control_points_mm } => {
                push_u8(bytes, 14);
                push_u32(bytes, control_points_mm.len() as u32);
                for point in control_points_mm {
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
            FeatureKind::Pad(spec) => {
                push_u8(bytes, 19);
                push_u64(bytes, spec.sketch.0);
                push_u64(bytes, spec.region.0);
                write_feature_direction(bytes, spec.direction);
                write_feature_extent(bytes, &spec.extent);
            }
            FeatureKind::SketchPocket(spec) => {
                push_u8(bytes, 20);
                push_u64(bytes, spec.target.0);
                push_u64(bytes, spec.sketch.0);
                push_u64(bytes, spec.region.0);
                write_feature_direction(bytes, spec.direction);
                write_feature_extent(bytes, &spec.extent);
                write_exact_reference(bytes, &spec.support);
            }
            FeatureKind::ThroughCut { target, profile } => {
                push_u8(bytes, 3);
                push_u64(bytes, target.0);
                push_u64(bytes, profile.0);
            }
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => {
                push_u8(bytes, 10);
                push_u64(bytes, target.0);
                push_u64(bytes, profile.0);
                push_string(bytes, depth.source_token());
                push_u64(bytes, depth.millimetres().to_bits());
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
                        BooleanOperation::Intersect => 3,
                        BooleanOperation::Split => 4,
                    },
                );
                push_u64(bytes, target.0);
                push_u64(bytes, tool.0);
            }
            FeatureKind::PlanarOffset { profile, distance } => {
                push_u8(bytes, 12);
                push_u64(bytes, profile.0);
                push_string(bytes, distance.source_token());
                push_u64(bytes, distance.millimetres().to_bits());
            }
            FeatureKind::Sweep { profile, path } => {
                push_u8(bytes, 13);
                push_u64(bytes, profile.0);
                push_u64(bytes, path.0);
            }
            FeatureKind::Loft { sections } => {
                push_u8(bytes, 15);
                push_u32(bytes, sections.len() as u32);
                for section in sections {
                    push_u64(bytes, section.profile.0);
                    push_u64(bytes, section.elevation_mm.to_bits());
                }
            }
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                push_u8(bytes, 4);
                push_u64(bytes, profile.0);
                for coordinate in axis_start_mm.iter().chain(axis_end_mm) {
                    push_u64(bytes, coordinate.to_bits());
                }
                push_u64(bytes, angle_degrees.to_bits());
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
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
            } => {
                push_u8(bytes, 5);
                push_u64(bytes, target.0);
                push_u32(bytes, removed_faces.len() as u32);
                for role in removed_faces {
                    push_string(bytes, role.as_str());
                }
                push_string(bytes, thickness.source_token());
                push_u64(bytes, thickness.millimetres().to_bits());
            }
            FeatureKind::BottleEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                push_u8(bytes, 7);
                push_u64(bytes, target.0);
                push_u32(bytes, edges.len() as u32);
                for role in edges {
                    push_string(bytes, role.as_str());
                }
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
            FeatureKind::TopologyShell {
                target,
                removed_faces,
                thickness,
            } => {
                push_u8(bytes, 21);
                push_u64(bytes, target.0);
                push_u32(bytes, removed_faces.len() as u32);
                for reference in removed_faces {
                    push_topological_reference(bytes, reference);
                }
                push_string(bytes, thickness.source_token());
                push_u64(bytes, thickness.millimetres().to_bits());
            }
            FeatureKind::TopologyEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                push_u8(bytes, 22);
                push_u64(bytes, target.0);
                push_u32(bytes, edges.len() as u32);
                for reference in edges {
                    push_topological_reference(bytes, reference);
                }
                push_u8(
                    bytes,
                    match kind {
                        EdgeFinishKind::Fillet => 1,
                        EdgeFinishKind::Chamfer => 2,
                    },
                );
                push_string(bytes, amount.source_token());
                push_u64(bytes, amount.millimetres().to_bits());
            }
            FeatureKind::TopologyFaceOffset {
                target,
                face,
                distance,
            } => {
                push_u8(bytes, 23);
                push_u64(bytes, target.0);
                push_topological_reference(bytes, face);
                push_string(bytes, distance.source_token());
                push_u64(bytes, distance.millimetres().to_bits());
            }
            FeatureKind::RigidTransform { target, transform } => {
                push_u8(bytes, 24);
                push_u64(bytes, target.0);
                for value in transform.matrix() {
                    push_u64(bytes, value.to_bits());
                }
            }
            FeatureKind::ImportedExactBody(spec) => {
                push_u8(bytes, 16);
                push_string(bytes, &spec.schema);
                push_u64(bytes, spec.import_id.0);
                bytes.extend_from_slice(&spec.source_sha256);
                push_u64(bytes, spec.source_byte_len);
                push_string(bytes, &spec.result_fingerprint);
                push_u32(bytes, spec.solid_count);
                match spec.topology_counts {
                    Some(topology_counts) => {
                        push_u8(bytes, 1);
                        for count in topology_counts {
                            push_u32(bytes, count);
                        }
                    }
                    None => push_u8(bytes, 0),
                }
                push_u64(bytes, spec.volume_mm3.to_bits());
                for coordinate in spec.bounds_mm.iter().flatten() {
                    push_u64(bytes, coordinate.to_bits());
                }
                push_string(bytes, &spec.backend);
                push_string(bytes, &spec.tolerance);
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
                    MeshAuthority::ImportedStl { import_id } => {
                        push_u8(bytes, 3);
                        push_u64(bytes, import_id.0);
                    }
                    MeshAuthority::ImportedSketchupScene { import_id } => {
                        push_u8(bytes, 4);
                        push_u64(bytes, import_id.0);
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

fn write_assembly_mate(bytes: &mut Vec<u8>, mate: &AssemblyMate) {
    push_string(bytes, mate.schema());
    push_u64(bytes, mate.id().0);
    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
        push_u64(bytes, endpoint.occurrence_id().0);
        write_exact_reference(bytes, endpoint.reference());
        match endpoint.health() {
            AssemblyReferenceHealth::Resolved => push_u8(bytes, 1),
            AssemblyReferenceHealth::Ambiguous { candidate_count } => {
                push_u8(bytes, 2);
                push_u32(bytes, candidate_count);
            }
            AssemblyReferenceHealth::Lost => push_u8(bytes, 3),
            AssemblyReferenceHealth::Broken => push_u8(bytes, 4),
        }
    }
    match mate.kind() {
        AssemblyMateKind::CoincidentPlanar {
            offset_mm,
            reversed,
        } => {
            push_u8(bytes, 1);
            push_u64(bytes, offset_mm.to_bits());
            push_u8(bytes, u8::from(reversed));
        }
        AssemblyMateKind::ConcentricAxial { reversed } => {
            push_u8(bytes, 2);
            push_u8(bytes, u8::from(reversed));
        }
        AssemblyMateKind::Distance { distance_mm } => {
            push_u8(bytes, 3);
            push_u64(bytes, distance_mm.to_bits());
        }
        AssemblyMateKind::Angle { angle_degrees } => {
            push_u8(bytes, 4);
            push_u64(bytes, angle_degrees.to_bits());
        }
    }
}

fn write_assembly_joint(bytes: &mut Vec<u8>, joint: &AssemblyJoint) {
    push_string(bytes, joint.schema());
    push_u64(bytes, joint.id().0);
    push_u64(bytes, joint.parent_occurrence_id().0);
    push_u64(bytes, joint.child_occurrence_id().0);
    match joint.kind() {
        AssemblyJointKind::Fixed => push_u8(bytes, 1),
        AssemblyJointKind::Revolute {
            axis,
            limits,
            position_degrees,
        } => {
            push_u8(bytes, 2);
            write_assembly_joint_axis(bytes, axis);
            write_assembly_joint_limits(bytes, limits);
            push_u64(bytes, position_degrees.to_bits());
        }
        AssemblyJointKind::Prismatic {
            axis,
            limits,
            position_mm,
        } => {
            push_u8(bytes, 3);
            write_assembly_joint_axis(bytes, axis);
            write_assembly_joint_limits(bytes, limits);
            push_u64(bytes, position_mm.to_bits());
        }
    }
}

fn write_assembly_joint_axis(bytes: &mut Vec<u8>, axis: AssemblyJointAxis) {
    for value in axis.direction_in_parent() {
        push_u64(bytes, value.to_bits());
    }
    for value in axis.pivot_in_parent_mm() {
        push_u64(bytes, value.to_bits());
    }
}

fn write_assembly_joint_limits(bytes: &mut Vec<u8>, limits: Option<AssemblyJointLimits>) {
    push_u8(bytes, u8::from(limits.is_some()));
    if let Some(limits) = limits {
        push_u64(bytes, limits.min().to_bits());
        push_u64(bytes, limits.max().to_bits());
    }
}

fn write_mechanical_interface(bytes: &mut Vec<u8>, interface: &MechanicalInterface) {
    push_string(bytes, interface.schema());
    push_u64(bytes, interface.id().0);
    push_u64(bytes, interface.occurrence_id().0);
    push_u8(
        bytes,
        match interface.role() {
            MechanicalRole::Mounting => 1,
            MechanicalRole::Support => 2,
            MechanicalRole::Guide => 3,
        },
    );
    push_u32(bytes, interface.face_ordinal());
    push_string(bytes, interface.geometry_fingerprint());
    let frame = interface.frame();
    for value in frame.origin_mm() {
        push_u64(bytes, value.to_bits());
    }
    for value in frame.normal() {
        push_u64(bytes, value.to_bits());
    }
    push_u64(bytes, frame.area_mm2().to_bits());
    for corner in frame.bounds_mm() {
        for value in corner {
            push_u64(bytes, value.to_bits());
        }
    }
}

fn write_mechanical_condition(bytes: &mut Vec<u8>, condition: &MechanicalCondition) {
    push_string(bytes, condition.schema());
    push_u64(bytes, condition.id().0);
    match condition.kind() {
        MechanicalConditionKind::PlanarContact {
            first,
            second,
            offset_mm,
            tolerance_mm,
        } => {
            push_u8(bytes, 1);
            push_u64(bytes, first.0);
            push_u64(bytes, second.0);
            push_u64(bytes, offset_mm.to_bits());
            push_u64(bytes, tolerance_mm.to_bits());
        }
        MechanicalConditionKind::Support {
            supported,
            supporting,
            tolerance_mm,
        } => {
            push_u8(bytes, 2);
            push_u64(bytes, supported.0);
            push_u64(bytes, supporting.0);
            push_u64(bytes, tolerance_mm.to_bits());
        }
        MechanicalConditionKind::JointAxisAlignment {
            joint_id,
            interface,
            alignment,
            tolerance_degrees,
        } => {
            push_u8(bytes, 3);
            push_u64(bytes, joint_id.0);
            push_u64(bytes, interface.0);
            push_u8(
                bytes,
                match alignment {
                    MechanicalAxisAlignment::Parallel => 1,
                    MechanicalAxisAlignment::Perpendicular => 2,
                },
            );
            push_u64(bytes, tolerance_degrees.to_bits());
        }
        MechanicalConditionKind::JointTravel {
            joint_id,
            minimum,
            maximum,
        } => {
            push_u8(bytes, 4);
            push_u64(bytes, joint_id.0);
            push_u64(bytes, minimum.to_bits());
            push_u64(bytes, maximum.to_bits());
        }
    }
}

fn write_assembly_motion_coupling(bytes: &mut Vec<u8>, coupling: &AssemblyMotionCoupling) {
    push_string(bytes, coupling.schema());
    push_u64(bytes, coupling.id().0);
    push_u64(bytes, coupling.input_joint_id().0);
    push_u64(bytes, coupling.output_joint_id().0);
    push_u64(bytes, coupling.input_reference_position().to_bits());
    push_u64(bytes, coupling.output_reference_position().to_bits());
    match coupling.transmission() {
        AssemblyTransmissionKind::GearPair {
            input_teeth,
            output_teeth,
            mesh,
        } => {
            push_u8(bytes, 1);
            push_u32(bytes, input_teeth);
            push_u32(bytes, output_teeth);
            push_u8(
                bytes,
                match mesh {
                    GearMeshKind::External => 1,
                    GearMeshKind::Internal => 2,
                },
            );
        }
        AssemblyTransmissionKind::Belt {
            input_pitch_diameter_mm,
            output_pitch_diameter_mm,
            crossed,
        } => {
            push_u8(bytes, 2);
            push_u64(bytes, input_pitch_diameter_mm.to_bits());
            push_u64(bytes, output_pitch_diameter_mm.to_bits());
            push_u8(bytes, u8::from(crossed));
        }
        AssemblyTransmissionKind::Chain {
            input_sprocket_teeth,
            output_sprocket_teeth,
        } => {
            push_u8(bytes, 3);
            push_u32(bytes, input_sprocket_teeth);
            push_u32(bytes, output_sprocket_teeth);
        }
        AssemblyTransmissionKind::RackAndPinion {
            pinion_pitch_diameter_mm,
            direction,
        } => {
            push_u8(bytes, 4);
            push_u64(bytes, pinion_pitch_diameter_mm.to_bits());
            push_u8(
                bytes,
                match direction {
                    AssemblyMotionDirection::Same => 1,
                    AssemblyMotionDirection::Opposite => 2,
                },
            );
        }
        AssemblyTransmissionKind::LeadScrew {
            lead_mm_per_revolution,
            handedness,
        } => {
            push_u8(bytes, 5);
            push_u64(bytes, lead_mm_per_revolution.to_bits());
            push_u8(
                bytes,
                match handedness {
                    ScrewHandedness::Right => 1,
                    ScrewHandedness::Left => 2,
                },
            );
        }
    }
}

fn write_assembly_motion_study(bytes: &mut Vec<u8>, study: &AssemblyMotionStudy) {
    push_string(bytes, study.schema());
    push_u64(bytes, study.id().0);
    push_string(bytes, study.name());
    push_u32(bytes, study.drivers().len() as u32);
    for driver in study.drivers() {
        push_u64(bytes, driver.joint_id().0);
        push_u64(bytes, driver.position().to_bits());
    }
}

fn write_drawing_sheet(bytes: &mut Vec<u8>, sheet: &DrawingSheet) {
    push_string(bytes, sheet.schema());
    push_u64(bytes, sheet.id().0);
    push_string(bytes, sheet.name());
    match sheet.source() {
        DrawingSource::Definition(id) => {
            push_u8(bytes, 1);
            push_u64(bytes, id.0);
        }
        DrawingSource::RigidAssembly { occurrence_ids } => {
            push_u8(bytes, 2);
            write_ids(bytes, occurrence_ids.iter().map(|id| id.0));
        }
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
    mut container_data: ContainerData,
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
            | SPACE_CLEARANCE_SCHEMA
            | POCKET_SCHEMA
            | SEGMENT_PROFILE_SCHEMA
            | GENERAL_REVOLVE_SCHEMA
            | STABLE_SUBSHAPE_ROLE_SCHEMA
            | BOOLEAN_INTERSECT_SCHEMA
            | BOOLEAN_SPLIT_SCHEMA
            | PLANAR_OFFSET_SCHEMA
            | SWEEP_SCHEMA
            | LOFT_SPLINE_SCHEMA
            | IMPORT_RECEIPT_SCHEMA
            | IMPORTED_EXACT_BODY_SCHEMA
            | SKETCHUP_SCENE_SCHEMA
            | WORKPLANE_SKETCH_SCHEMA
            | ASSEMBLY_CONTRACT_SCHEMA
            | ORTHOGRAPHIC_DRAWING_SCHEMA
            | BODY_CONTRACT_SCHEMA
            | BODY_CONSUMPTION_SCHEMA
            | BODY_FEATURE_SUPPRESSION_SCHEMA
            | CLASSIFICATION_DIMENSION_SCHEMA
            | FEATURE_EXTENT_SCHEMA
            | IMPORTED_TOPOLOGY_COUNTS_SCHEMA
            | TOPOLOGICAL_FEATURE_REFERENCE_SCHEMA
            | GENERAL_PARAMETER_PATH_SCHEMA
            | ASSEMBLY_KINEMATICS_SCHEMA
            | ASSEMBLY_MOTION_COUPLING_SCHEMA
            | MECHANICAL_CONTRACT_SCHEMA
            | SKETCH_CONSTRAINT_VOCABULARY_SCHEMA
            | RIGID_TRANSFORM_FEATURE_SCHEMA
            | CUBIC_BEZIER_SKETCH_SCHEMA
            | CUBIC_BEZIER_SEGMENT_PROFILE_SCHEMA
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
            let mut product = ProductModel::default();
            let source_digest = crate::graph::sha256_bytes(bytes);
            product.document_id = crate::document::DocumentId(
                u64::from_le_bytes(source_digest[..8].try_into().expect("SHA-256 prefix")).max(1),
            );
            product
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
    let imported_sources = loaded_snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some((
                spec.source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
                spec.source_byte_len,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (hash, byte_len) in imported_sources {
        if !container_data
            .blobs
            .get(&hash)
            .is_some_and(|bytes| bytes.len() as u64 == byte_len)
        {
            return Err(PersistenceError::InvalidBlobHash);
        }
        Arc::make_mut(&mut container_data.imported_source_blobs).insert(hash);
    }
    for reference in loaded_snapshot.exact_reference_evidence() {
        let matches_current_request =
            crate::exact_product::ExactFeatureChainRequest::from_snapshot_for_producer(
                &loaded_snapshot,
                reference.definition_id,
                reference.producer_feature_id,
            )
            .is_ok_and(|request| {
                reference.matches_request(&request) || reference.matches_legacy_request(&request)
            }) || crate::exact_revolve::ExactRevolveRequest::from_snapshot(
                &loaded_snapshot,
                reference.definition_id,
            )
            .is_ok_and(|request| {
                crate::exact_revolve::reference_matches_revolve_request(reference, &request)
            });
        let matches_durable_anchor =
            crate::exact_product::ExactFeatureChainRequest::from_snapshot_for_producer(
                &loaded_snapshot,
                reference.definition_id,
                reference.producer_feature_id,
            )
            .is_ok_and(|request| reference.matches_durable_request_identity(&request))
                && loaded_snapshot.features().any(|feature| {
                    matches!(
                        feature.kind(),
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace { reference: support, .. },
                            ..
                        }) if support.as_ref() == reference
                    )
                });
        if !matches_current_request && !matches_durable_anchor {
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

fn read_legacy_parameter_path(reader: &mut Reader<'_>) -> Result<ParameterPath, PersistenceError> {
    let path = match reader.u8()? {
        1 => "height",
        2 => "body_radius",
        3 => "body_height",
        4 => "shoulder_rise",
        5 => "thickness",
        6 => "amount",
        7 => "bounds.width",
        8 => "bounds.height",
        value => return Err(PersistenceError::InvalidParameterSlot(value)),
    };
    ParameterPath::new(path).map_err(|_| PersistenceError::InvalidParameterPath)
}

fn read_feature_parameter_target(
    reader: &mut Reader<'_>,
    general_parameter_paths: bool,
) -> Result<FeatureParameterTarget, PersistenceError> {
    let feature_id = FeatureId(reader.u64()?);
    let (path, value_type) = if general_parameter_paths {
        let path = ParameterPath::new(reader.string()?)
            .map_err(|_| PersistenceError::InvalidParameterPath)?;
        let value_type = match reader.u8()? {
            1 => ParameterValueType::Length,
            2 => ParameterValueType::Angle,
            3 => ParameterValueType::Scalar,
            value => return Err(PersistenceError::InvalidParameterValueType(value)),
        };
        (path, value_type)
    } else {
        (
            read_legacy_parameter_path(reader)?,
            ParameterValueType::Length,
        )
    };
    Ok(FeatureParameterTarget {
        feature_id,
        path,
        value_type,
    })
}

fn read_persistent_dimension(
    reader: &mut Reader<'_>,
    general_parameter_paths: bool,
) -> Result<PersistentDimension, PersistenceError> {
    let id = PersistentDimensionId(reader.u64()?);
    let name = reader.string()?;
    let target = match reader.u8()? {
        1 => PersistentDimensionTarget::FeatureParameter(read_feature_parameter_target(
            reader,
            general_parameter_paths,
        )?),
        2 => PersistentDimensionTarget::DerivedOutput(read_identity(reader)?),
        3 => {
            let definition_id = DefinitionId(reader.u64()?);
            if general_parameter_paths {
                let target = read_feature_parameter_target(reader, true)?;
                PersistentDimensionTarget::ExactFeatureParameter {
                    definition_id,
                    producer_feature_id: target.feature_id,
                    semantic_role: reader.string()?,
                    source_element_id: reader.string()?,
                    path: target.path,
                    value_type: target.value_type,
                }
            } else {
                let producer_feature_id = FeatureId(reader.u64()?);
                let semantic_role = reader.string()?;
                let source_element_id = reader.string()?;
                PersistentDimensionTarget::ExactFeatureParameter {
                    definition_id,
                    producer_feature_id,
                    semantic_role,
                    source_element_id,
                    path: read_legacy_parameter_path(reader)?,
                    value_type: ParameterValueType::Length,
                }
            }
        }
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
    general_parameter_paths: bool,
) -> Result<FeatureParameterBinding, PersistenceError> {
    Ok(FeatureParameterBinding {
        target: read_feature_parameter_target(reader, general_parameter_paths)?,
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

fn read_topological_reference(
    reader: &mut Reader<'_>,
) -> Result<TopologicalElementRef, PersistenceError> {
    let length = usize::try_from(reader.count_with_limit(128 * 1024)?)
        .map_err(|_| PersistenceError::LengthOverflow)?;
    TopologicalElementRef::from_bytes(reader.take(length)?).map_err(|_| {
        PersistenceError::InvalidCanonicalData(CanonicalError::InvalidTopologicalFeatureReference)
    })
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

fn read_feature_direction(reader: &mut Reader<'_>) -> Result<FeatureDirection, PersistenceError> {
    match reader.u8()? {
        1 => Ok(FeatureDirection::AlongNormal),
        2 => Ok(FeatureDirection::OppositeNormal),
        3 => Ok(FeatureDirection::Vector([
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
        ])),
        value => Err(PersistenceError::InvalidFeatureKind(value)),
    }
}

fn read_extent_dimension(reader: &mut Reader<'_>) -> Result<Dimension, PersistenceError> {
    Ok(Dimension::new(
        reader.string()?,
        f64::from_bits(reader.u64()?),
    )?)
}

fn read_feature_extent_end(reader: &mut Reader<'_>) -> Result<FeatureExtentEnd, PersistenceError> {
    match reader.u8()? {
        1 => Ok(FeatureExtentEnd::Blind(read_extent_dimension(reader)?)),
        2 => Ok(FeatureExtentEnd::ThroughAll),
        3 => Ok(FeatureExtentEnd::UpToFace(Box::new(read_exact_reference(
            reader,
        )?))),
        value => Err(PersistenceError::InvalidFeatureKind(value)),
    }
}

fn read_feature_extent(reader: &mut Reader<'_>) -> Result<FeatureExtent, PersistenceError> {
    match reader.u8()? {
        1 => Ok(FeatureExtent::Blind(read_extent_dimension(reader)?)),
        2 => Ok(FeatureExtent::ThroughAll),
        3 => Ok(FeatureExtent::UpToFace(Box::new(read_exact_reference(
            reader,
        )?))),
        4 => Ok(FeatureExtent::Symmetric(read_extent_dimension(reader)?)),
        5 => Ok(FeatureExtent::Bidirectional {
            along: read_feature_extent_end(reader)?,
            opposite: read_feature_extent_end(reader)?,
        }),
        value => Err(PersistenceError::InvalidFeatureKind(value)),
    }
}

fn write_import_receipt(bytes: &mut Vec<u8>, receipt: &ImportReceipt) {
    push_u64(bytes, receipt.id().0);
    push_u8(
        bytes,
        match receipt.format() {
            ImportFormat::Stl => 1,
            ImportFormat::Dxf => 2,
            ImportFormat::Step => 3,
            ImportFormat::SketchupScene => 4,
        },
    );
    bytes.extend_from_slice(receipt.source_sha256());
    push_u64(bytes, receipt.source_byte_len());
    push_string(bytes, receipt.source_name());
    push_u8(
        bytes,
        match receipt.units().source_unit() {
            ImportLengthUnit::Millimetre => 1,
            ImportLengthUnit::Centimetre => 2,
            ImportLengthUnit::Metre => 3,
            ImportLengthUnit::Inch => 4,
            ImportLengthUnit::Foot => 5,
        },
    );
    push_u8(
        bytes,
        match receipt.units().authority() {
            ImportUnitAuthority::FileDeclared => 1,
            ImportUnitAuthority::UserDeclared => 2,
        },
    );
    push_string(bytes, receipt.parser_id());
    push_string(bytes, receipt.parser_version());
    push_u32(bytes, receipt.diagnostics().len() as u32);
    for diagnostic in receipt.diagnostics() {
        push_u8(
            bytes,
            match diagnostic.severity() {
                ImportDiagnosticSeverity::Info => 1,
                ImportDiagnosticSeverity::Warning => 2,
            },
        );
        push_string(bytes, diagnostic.code());
        match diagnostic.subject() {
            Some(subject) => {
                push_u8(bytes, 1);
                push_string(bytes, subject);
            }
            None => push_u8(bytes, 0),
        }
        push_u32(bytes, diagnostic.count());
    }
    push_u32(bytes, receipt.outputs().len() as u32);
    for output in receipt.outputs() {
        match output {
            ImportOutputRef::Definition(id) => {
                push_u8(bytes, 1);
                push_u64(bytes, id.0);
            }
            ImportOutputRef::Feature(id) => {
                push_u8(bytes, 2);
                push_u64(bytes, id.0);
            }
            ImportOutputRef::Occurrence(id) => {
                push_u8(bytes, 3);
                push_u64(bytes, id.0);
            }
        }
    }
}

fn read_import_receipt(
    reader: &mut Reader<'_>,
    sketchup_scene: bool,
) -> Result<ImportReceipt, PersistenceError> {
    let id = ImportId(reader.u64()?);
    let format = match reader.u8()? {
        1 => ImportFormat::Stl,
        2 => ImportFormat::Dxf,
        3 => ImportFormat::Step,
        4 if sketchup_scene => ImportFormat::SketchupScene,
        value => return Err(PersistenceError::InvalidImportFormat(value)),
    };
    let source_sha256 = reader
        .take(32)?
        .try_into()
        .map_err(|_| PersistenceError::Truncated)?;
    let source_byte_len = reader.u64()?;
    let source_name = reader.string()?;
    let source_unit = match reader.u8()? {
        1 => ImportLengthUnit::Millimetre,
        2 => ImportLengthUnit::Centimetre,
        3 => ImportLengthUnit::Metre,
        4 => ImportLengthUnit::Inch,
        5 => ImportLengthUnit::Foot,
        value => return Err(PersistenceError::InvalidImportUnit(value)),
    };
    let authority = match reader.u8()? {
        1 => ImportUnitAuthority::FileDeclared,
        2 => ImportUnitAuthority::UserDeclared,
        value => return Err(PersistenceError::InvalidImportUnit(value)),
    };
    let parser_id = reader.string()?;
    let parser_version = reader.string()?;
    let mut diagnostics = Vec::new();
    for _ in 0..reader.count_with_limit(MAX_IMPORT_DIAGNOSTICS as u32)? {
        let severity = match reader.u8()? {
            1 => ImportDiagnosticSeverity::Info,
            2 => ImportDiagnosticSeverity::Warning,
            value => return Err(PersistenceError::InvalidImportDiagnostic(value)),
        };
        let code = reader.string()?;
        let subject = match reader.u8()? {
            0 => None,
            1 => Some(reader.string()?),
            value => return Err(PersistenceError::InvalidOptionalMarker(value)),
        };
        diagnostics.push(
            ImportDiagnostic::new(severity, code, subject, reader.u32()?).map_err(|_| {
                PersistenceError::InvalidCanonicalData(CanonicalError::InvalidImportReceipt)
            })?,
        );
    }
    let mut outputs = Vec::new();
    for _ in 0..reader.count_with_limit(MAX_IMPORT_OUTPUTS as u32)? {
        outputs.push(match reader.u8()? {
            1 => ImportOutputRef::Definition(DefinitionId(reader.u64()?)),
            2 => ImportOutputRef::Feature(FeatureId(reader.u64()?)),
            3 => ImportOutputRef::Occurrence(OccurrenceId(reader.u64()?)),
            value => return Err(PersistenceError::InvalidImportOutput(value)),
        });
    }
    ImportReceipt::new(
        id,
        format,
        source_sha256,
        source_byte_len,
        source_name,
        ImportUnitDecision::new(source_unit, authority),
        parser_id,
        parser_version,
        diagnostics,
        outputs,
    )
    .map_err(|_| PersistenceError::InvalidCanonicalData(CanonicalError::InvalidImportReceipt))
}

fn read_workplane(reader: &mut Reader<'_>) -> Result<WorkplaneSpec, PersistenceError> {
    let support = match reader.u8()? {
        1 => WorkplaneSupport::Principal(match reader.u8()? {
            1 => PrincipalPlane::Xy,
            2 => PrincipalPlane::Yz,
            3 => PrincipalPlane::Xz,
            value => return Err(PersistenceError::InvalidFeatureKind(value)),
        }),
        2 => WorkplaneSupport::Offset {
            base: FeatureId(reader.u64()?),
            distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
        },
        3 => WorkplaneSupport::PlanarFace {
            reference: Box::new(read_exact_reference(reader)?),
            health: match reader.u8()? {
                1 => WorkplaneSupportHealth::Resolved,
                2 => WorkplaneSupportHealth::Ambiguous,
                3 => WorkplaneSupportHealth::Lost,
                4 => WorkplaneSupportHealth::Stale,
                value => return Err(PersistenceError::InvalidFeatureKind(value)),
            },
        },
        value => return Err(PersistenceError::InvalidFeatureKind(value)),
    };
    let point = |reader: &mut Reader<'_>| -> Result<[f64; 3], PersistenceError> {
        Ok([
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
        ])
    };
    Ok(WorkplaneSpec {
        support,
        frame: WorkplaneFrame {
            origin_mm: point(reader)?,
            x_axis: point(reader)?,
            y_axis: point(reader)?,
            normal: point(reader)?,
        },
    })
}

fn read_sketch_point_ref(
    reader: &mut Reader<'_>,
    cubic_bezier_sketch: bool,
) -> Result<SketchPointRef, PersistenceError> {
    Ok(SketchPointRef {
        entity: SketchEntityId(reader.u64()?),
        point: match reader.u8()? {
            1 => SketchPointKind::Start,
            2 => SketchPointKind::End,
            3 => SketchPointKind::Center,
            4 if cubic_bezier_sketch => SketchPointKind::Control1,
            5 if cubic_bezier_sketch => SketchPointKind::Control2,
            value => return Err(PersistenceError::InvalidFeatureKind(value)),
        },
    })
}

fn read_sketch(
    reader: &mut Reader<'_>,
    full_constraint_vocabulary: bool,
    cubic_bezier_sketch: bool,
) -> Result<SketchSpec, PersistenceError> {
    let workplane = FeatureId(reader.u64()?);
    let point = |reader: &mut Reader<'_>| -> Result<[f64; 2], PersistenceError> {
        Ok([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)])
    };
    let mut entities = Vec::new();
    for _ in 0..reader.count_with_limit(MAX_SKETCH_ENTITIES as u32)? {
        entities.push(match reader.u8()? {
            1 => SketchEntity::Line {
                id: SketchEntityId(reader.u64()?),
                start_mm: point(reader)?,
                end_mm: point(reader)?,
            },
            2 => SketchEntity::Arc {
                id: SketchEntityId(reader.u64()?),
                start_mm: point(reader)?,
                end_mm: point(reader)?,
                center_mm: point(reader)?,
                clockwise: reader.boolean()?,
            },
            3 => SketchEntity::Circle {
                id: SketchEntityId(reader.u64()?),
                center_mm: point(reader)?,
                radius_mm: f64::from_bits(reader.u64()?),
            },
            4 if cubic_bezier_sketch => SketchEntity::CubicBezier {
                id: SketchEntityId(reader.u64()?),
                start_mm: point(reader)?,
                control_1_mm: point(reader)?,
                control_2_mm: point(reader)?,
                end_mm: point(reader)?,
            },
            value => return Err(PersistenceError::InvalidFeatureKind(value)),
        });
    }
    let mut constraints = Vec::new();
    for _ in 0..reader.count_with_limit(MAX_SKETCH_CONSTRAINTS as u32)? {
        let id = SketchConstraintId(reader.u64()?);
        let kind = match reader.u8()? {
            1 => SketchConstraintKind::Horizontal {
                entity: SketchEntityId(reader.u64()?),
            },
            2 => SketchConstraintKind::Vertical {
                entity: SketchEntityId(reader.u64()?),
            },
            3 => SketchConstraintKind::Coincident {
                a: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                b: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
            },
            4 => SketchConstraintKind::Distance {
                a: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                b: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                value: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            5 => SketchConstraintKind::Radius {
                entity: SketchEntityId(reader.u64()?),
                value: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            6 => SketchConstraintKind::FixedPoint {
                point: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                position_mm: point(reader)?,
            },
            7 if full_constraint_vocabulary => SketchConstraintKind::Parallel {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
            },
            8 if full_constraint_vocabulary => SketchConstraintKind::Perpendicular {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
            },
            9 if full_constraint_vocabulary => SketchConstraintKind::Tangent {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
            },
            10 if full_constraint_vocabulary => SketchConstraintKind::Angle {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
                angle_degrees: f64::from_bits(reader.u64()?),
            },
            11 if full_constraint_vocabulary => SketchConstraintKind::Equal {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
            },
            12 if full_constraint_vocabulary => SketchConstraintKind::Symmetric {
                a: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                b: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                axis: SketchEntityId(reader.u64()?),
            },
            13 if full_constraint_vocabulary => SketchConstraintKind::Concentric {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
            },
            14 if full_constraint_vocabulary => SketchConstraintKind::Collinear {
                a: SketchEntityId(reader.u64()?),
                b: SketchEntityId(reader.u64()?),
            },
            15 if full_constraint_vocabulary => SketchConstraintKind::Midpoint {
                point: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                line: SketchEntityId(reader.u64()?),
            },
            16 if full_constraint_vocabulary => SketchConstraintKind::PointOnCurve {
                point: read_sketch_point_ref(reader, cubic_bezier_sketch)?,
                curve: SketchEntityId(reader.u64()?),
            },
            value => return Err(PersistenceError::InvalidFeatureKind(value)),
        };
        constraints.push(SketchConstraint { id, kind });
    }
    Ok(SketchSpec {
        workplane,
        entities,
        constraints,
    })
}

fn read_assembly_mate(reader: &mut Reader<'_>) -> Result<AssemblyMate, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_MATE_SCHEMA_V1 {
        return Err(PersistenceError::InvalidAssemblyMate);
    }
    let id = AssemblyMateId(reader.u64()?);
    let mut endpoint = || -> Result<AssemblyMateEndpoint, PersistenceError> {
        let occurrence_id = OccurrenceId(reader.u64()?);
        let reference = read_exact_reference(reader)?;
        let health = match reader.u8()? {
            1 => AssemblyReferenceHealth::Resolved,
            2 => AssemblyReferenceHealth::Ambiguous {
                candidate_count: reader.u32()?,
            },
            3 => AssemblyReferenceHealth::Lost,
            4 => AssemblyReferenceHealth::Broken,
            _ => return Err(PersistenceError::InvalidAssemblyMate),
        };
        Ok(AssemblyMateEndpoint {
            occurrence_id,
            reference,
            health,
        })
    };
    let endpoint_a = endpoint()?;
    let endpoint_b = endpoint()?;
    let kind = match reader.u8()? {
        1 => AssemblyMateKind::CoincidentPlanar {
            offset_mm: f64::from_bits(reader.u64()?),
            reversed: reader.boolean()?,
        },
        2 => AssemblyMateKind::ConcentricAxial {
            reversed: reader.boolean()?,
        },
        3 => AssemblyMateKind::Distance {
            distance_mm: f64::from_bits(reader.u64()?),
        },
        4 => AssemblyMateKind::Angle {
            angle_degrees: f64::from_bits(reader.u64()?),
        },
        _ => return Err(PersistenceError::InvalidAssemblyMate),
    };
    Ok(AssemblyMate {
        schema,
        id,
        endpoint_a: Box::new(endpoint_a),
        endpoint_b: Box::new(endpoint_b),
        kind,
    })
}

fn read_assembly_joint(reader: &mut Reader<'_>) -> Result<AssemblyJoint, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_JOINT_SCHEMA_V1 {
        return Err(PersistenceError::InvalidAssemblyJoint);
    }
    let id = AssemblyJointId(reader.u64()?);
    let parent_occurrence_id = OccurrenceId(reader.u64()?);
    let child_occurrence_id = OccurrenceId(reader.u64()?);
    let kind = match reader.u8()? {
        1 => AssemblyJointKind::Fixed,
        2 => AssemblyJointKind::Revolute {
            axis: read_assembly_joint_axis(reader)?,
            limits: read_assembly_joint_limits(reader)?,
            position_degrees: f64::from_bits(reader.u64()?),
        },
        3 => AssemblyJointKind::Prismatic {
            axis: read_assembly_joint_axis(reader)?,
            limits: read_assembly_joint_limits(reader)?,
            position_mm: f64::from_bits(reader.u64()?),
        },
        _ => return Err(PersistenceError::InvalidAssemblyJoint),
    };
    Ok(AssemblyJoint {
        schema,
        id,
        parent_occurrence_id,
        child_occurrence_id,
        kind,
    })
}

fn read_assembly_joint_axis(
    reader: &mut Reader<'_>,
) -> Result<AssemblyJointAxis, PersistenceError> {
    let mut direction_in_parent = [0.0; 3];
    let mut pivot_in_parent_mm = [0.0; 3];
    for value in &mut direction_in_parent {
        *value = f64::from_bits(reader.u64()?);
    }
    for value in &mut pivot_in_parent_mm {
        *value = f64::from_bits(reader.u64()?);
    }
    Ok(AssemblyJointAxis {
        direction_in_parent,
        pivot_in_parent_mm,
    })
}

fn read_assembly_joint_limits(
    reader: &mut Reader<'_>,
) -> Result<Option<AssemblyJointLimits>, PersistenceError> {
    if reader.boolean()? {
        Ok(Some(AssemblyJointLimits::new(
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
        )))
    } else {
        Ok(None)
    }
}

fn read_mechanical_interface(
    reader: &mut Reader<'_>,
) -> Result<MechanicalInterface, PersistenceError> {
    let schema = reader.string()?;
    if schema != MECHANICAL_INTERFACE_SCHEMA_V1 {
        return Err(PersistenceError::InvalidMechanicalInterface);
    }
    let id = MechanicalInterfaceId(reader.u64()?);
    let occurrence_id = OccurrenceId(reader.u64()?);
    let role = match reader.u8()? {
        1 => MechanicalRole::Mounting,
        2 => MechanicalRole::Support,
        3 => MechanicalRole::Guide,
        _ => return Err(PersistenceError::InvalidMechanicalInterface),
    };
    let face_ordinal = reader.u32()?;
    let geometry_fingerprint = reader.string()?;
    let origin_mm = read_vector3(reader)?;
    let normal = read_vector3(reader)?;
    let area_mm2 = f64::from_bits(reader.u64()?);
    let bounds_mm = [read_vector3(reader)?, read_vector3(reader)?];
    let interface = MechanicalInterface::new(
        id,
        occurrence_id,
        role,
        face_ordinal,
        geometry_fingerprint,
        MechanicalPlanarFrame::new(origin_mm, normal, area_mm2, bounds_mm),
    );
    if !interface.has_valid_shape() {
        return Err(PersistenceError::InvalidMechanicalInterface);
    }
    Ok(interface)
}

fn read_vector3(reader: &mut Reader<'_>) -> Result<[f64; 3], PersistenceError> {
    Ok([
        f64::from_bits(reader.u64()?),
        f64::from_bits(reader.u64()?),
        f64::from_bits(reader.u64()?),
    ])
}

fn read_mechanical_condition(
    reader: &mut Reader<'_>,
) -> Result<MechanicalCondition, PersistenceError> {
    let schema = reader.string()?;
    if schema != MECHANICAL_CONDITION_SCHEMA_V1 {
        return Err(PersistenceError::InvalidMechanicalCondition);
    }
    let id = MechanicalConditionId(reader.u64()?);
    let kind = match reader.u8()? {
        1 => MechanicalConditionKind::PlanarContact {
            first: MechanicalInterfaceId(reader.u64()?),
            second: MechanicalInterfaceId(reader.u64()?),
            offset_mm: f64::from_bits(reader.u64()?),
            tolerance_mm: f64::from_bits(reader.u64()?),
        },
        2 => MechanicalConditionKind::Support {
            supported: MechanicalInterfaceId(reader.u64()?),
            supporting: MechanicalInterfaceId(reader.u64()?),
            tolerance_mm: f64::from_bits(reader.u64()?),
        },
        3 => MechanicalConditionKind::JointAxisAlignment {
            joint_id: AssemblyJointId(reader.u64()?),
            interface: MechanicalInterfaceId(reader.u64()?),
            alignment: match reader.u8()? {
                1 => MechanicalAxisAlignment::Parallel,
                2 => MechanicalAxisAlignment::Perpendicular,
                _ => return Err(PersistenceError::InvalidMechanicalCondition),
            },
            tolerance_degrees: f64::from_bits(reader.u64()?),
        },
        4 => MechanicalConditionKind::JointTravel {
            joint_id: AssemblyJointId(reader.u64()?),
            minimum: f64::from_bits(reader.u64()?),
            maximum: f64::from_bits(reader.u64()?),
        },
        _ => return Err(PersistenceError::InvalidMechanicalCondition),
    };
    let condition = MechanicalCondition::new(id, kind);
    if !condition.has_valid_shape() {
        return Err(PersistenceError::InvalidMechanicalCondition);
    }
    Ok(condition)
}

fn read_assembly_motion_coupling(
    reader: &mut Reader<'_>,
) -> Result<AssemblyMotionCoupling, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_MOTION_COUPLING_SCHEMA_V1 {
        return Err(PersistenceError::InvalidAssemblyMotionCoupling);
    }
    let id = AssemblyMotionCouplingId(reader.u64()?);
    let input_joint_id = AssemblyJointId(reader.u64()?);
    let output_joint_id = AssemblyJointId(reader.u64()?);
    let input_reference_position = f64::from_bits(reader.u64()?);
    let output_reference_position = f64::from_bits(reader.u64()?);
    let transmission = match reader.u8()? {
        1 => AssemblyTransmissionKind::GearPair {
            input_teeth: reader.u32()?,
            output_teeth: reader.u32()?,
            mesh: match reader.u8()? {
                1 => GearMeshKind::External,
                2 => GearMeshKind::Internal,
                _ => return Err(PersistenceError::InvalidAssemblyMotionCoupling),
            },
        },
        2 => AssemblyTransmissionKind::Belt {
            input_pitch_diameter_mm: f64::from_bits(reader.u64()?),
            output_pitch_diameter_mm: f64::from_bits(reader.u64()?),
            crossed: reader.boolean()?,
        },
        3 => AssemblyTransmissionKind::Chain {
            input_sprocket_teeth: reader.u32()?,
            output_sprocket_teeth: reader.u32()?,
        },
        4 => AssemblyTransmissionKind::RackAndPinion {
            pinion_pitch_diameter_mm: f64::from_bits(reader.u64()?),
            direction: match reader.u8()? {
                1 => AssemblyMotionDirection::Same,
                2 => AssemblyMotionDirection::Opposite,
                _ => return Err(PersistenceError::InvalidAssemblyMotionCoupling),
            },
        },
        5 => AssemblyTransmissionKind::LeadScrew {
            lead_mm_per_revolution: f64::from_bits(reader.u64()?),
            handedness: match reader.u8()? {
                1 => ScrewHandedness::Right,
                2 => ScrewHandedness::Left,
                _ => return Err(PersistenceError::InvalidAssemblyMotionCoupling),
            },
        },
        _ => return Err(PersistenceError::InvalidAssemblyMotionCoupling),
    };
    Ok(AssemblyMotionCoupling {
        schema,
        id,
        input_joint_id,
        output_joint_id,
        input_reference_position,
        output_reference_position,
        transmission,
    })
}

fn read_assembly_motion_study(
    reader: &mut Reader<'_>,
) -> Result<AssemblyMotionStudy, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_MOTION_STUDY_SCHEMA_V1 {
        return Err(PersistenceError::InvalidAssemblyMotionStudy);
    }
    let id = AssemblyMotionStudyId(reader.u64()?);
    let name = reader.string()?;
    let mut drivers = Vec::new();
    for _ in 0..reader.count()? {
        drivers.push(AssemblyMotionDriver::new(
            AssemblyJointId(reader.u64()?),
            f64::from_bits(reader.u64()?),
        ));
    }
    Ok(AssemblyMotionStudy {
        schema,
        id,
        name,
        drivers,
    })
}

fn read_drawing_sheet(reader: &mut Reader<'_>) -> Result<DrawingSheet, PersistenceError> {
    if reader.string()? != ORTHOGRAPHIC_DRAWING_SCHEMA_V1 {
        return Err(PersistenceError::InvalidCanonicalData(
            CanonicalError::Drawing(crate::drawing::DrawingError::InvalidSheet),
        ));
    }
    let id = DrawingSheetId(reader.u64()?);
    let name = reader.string()?;
    let source = match reader.u8()? {
        1 => DrawingSource::Definition(DefinitionId(reader.u64()?)),
        2 => DrawingSource::RigidAssembly {
            occurrence_ids: read_ids(reader)?.into_iter().map(OccurrenceId).collect(),
        },
        _ => {
            return Err(PersistenceError::InvalidCanonicalData(
                CanonicalError::Drawing(crate::drawing::DrawingError::InvalidSheet),
            ));
        }
    };
    DrawingSheet::new(id, name, source)
        .map_err(|error| PersistenceError::InvalidCanonicalData(CanonicalError::Drawing(error)))
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
                let binding =
                    read_feature_parameter_binding(reader, capabilities.general_parameter_paths)?;
                let target = binding.target.clone();
                if product
                    .feature_parameter_bindings
                    .insert(target.clone(), Arc::new(binding))
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
        let name = reader.string()?;
        let feature_ids = read_ids(reader)?.into_iter().map(FeatureId).collect();
        let local_group_ids = if capabilities.current {
            read_ids(reader)?.into_iter().map(LocalGroupId).collect()
        } else {
            Vec::new()
        };
        let local_occurrence_ids = if capabilities.current {
            read_ids(reader)?
                .into_iter()
                .map(LocalOccurrenceId)
                .collect()
        } else {
            Vec::new()
        };
        let (bodies, active_body_id, feature_body_ownership) =
            (BTreeMap::new(), BodyId(1), BTreeMap::new());
        let definition = Definition {
            id,
            name,
            feature_ids,
            bodies,
            active_body_id,
            feature_body_ownership,
            local_group_ids,
            local_occurrence_ids,
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
            17 if capabilities.workplane_sketch => FeatureKind::Workplane(read_workplane(reader)?),
            18 if capabilities.workplane_sketch => FeatureKind::Sketch(read_sketch(
                reader,
                capabilities.sketch_constraint_vocabulary,
                capabilities.cubic_bezier_sketch,
            )?),
            1 => {
                let mut points_mm = Vec::new();
                for _ in 0..reader.count()? {
                    points_mm.push([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)]);
                }
                FeatureKind::Profile { points_mm }
            }
            11 if capabilities.segment_profile => {
                let closed = match reader.u8()? {
                    0 => false,
                    1 => true,
                    value => return Err(PersistenceError::InvalidFeatureKind(value)),
                };
                let mut segments = Vec::new();
                for _ in 0..reader.count()? {
                    let point = |reader: &mut Reader<'_>| -> Result<[f64; 2], PersistenceError> {
                        Ok([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)])
                    };
                    segments.push(match reader.u8()? {
                        1 => ProfileSegment::Line {
                            start_mm: point(reader)?,
                            end_mm: point(reader)?,
                        },
                        2 => ProfileSegment::CircularArc {
                            start_mm: point(reader)?,
                            end_mm: point(reader)?,
                            center_mm: point(reader)?,
                            clockwise: match reader.u8()? {
                                0 => false,
                                1 => true,
                                value => {
                                    return Err(PersistenceError::InvalidFeatureKind(value));
                                }
                            },
                        },
                        3 if capabilities.cubic_bezier_segment_profile => {
                            ProfileSegment::CubicBezier {
                                start_mm: point(reader)?,
                                control_1_mm: point(reader)?,
                                control_2_mm: point(reader)?,
                                end_mm: point(reader)?,
                            }
                        }
                        value => return Err(PersistenceError::InvalidFeatureKind(value)),
                    });
                }
                FeatureKind::SegmentProfile { segments, closed }
            }
            25 if capabilities.spatial_sweep_path => {
                let point = |reader: &mut Reader<'_>| -> Result<[f64; 3], PersistenceError> {
                    Ok([
                        f64::from_bits(reader.u64()?),
                        f64::from_bits(reader.u64()?),
                        f64::from_bits(reader.u64()?),
                    ])
                };
                let mut segments = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    segments.push(match reader.u8()? {
                        1 => SpatialPathSegment::Line {
                            start_mm: point(reader)?,
                            end_mm: point(reader)?,
                        },
                        2 => SpatialPathSegment::CircularArc {
                            start_mm: point(reader)?,
                            end_mm: point(reader)?,
                            center_mm: point(reader)?,
                            normal: point(reader)?,
                            clockwise: match reader.u8()? {
                                0 => false,
                                1 => true,
                                value => return Err(PersistenceError::InvalidFeatureKind(value)),
                            },
                        },
                        3 => SpatialPathSegment::CubicBezier {
                            start_mm: point(reader)?,
                            control_1_mm: point(reader)?,
                            control_2_mm: point(reader)?,
                            end_mm: point(reader)?,
                        },
                        value => return Err(PersistenceError::InvalidFeatureKind(value)),
                    });
                }
                FeatureKind::SpatialPath { segments }
            }
            14 if capabilities.loft_spline => {
                let mut control_points_mm = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    control_points_mm
                        .push([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)]);
                }
                FeatureKind::SplineProfile { control_points_mm }
            }
            2 => FeatureKind::Extrusion {
                profile: FeatureId(reader.u64()?),
                height: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            19 if capabilities.workplane_sketch => FeatureKind::Pad(PadSpec {
                sketch: FeatureId(reader.u64()?),
                region: SketchRegionId(reader.u64()?),
                direction: if capabilities.feature_extents {
                    read_feature_direction(reader)?
                } else {
                    match reader.u8()? {
                        1 => FeatureDirection::AlongNormal,
                        2 => FeatureDirection::OppositeNormal,
                        value => return Err(PersistenceError::InvalidFeatureKind(value)),
                    }
                },
                extent: if capabilities.feature_extents {
                    read_feature_extent(reader)?
                } else {
                    FeatureExtent::Blind(read_extent_dimension(reader)?)
                },
            }),
            20 if capabilities.workplane_sketch => FeatureKind::SketchPocket(PocketSpec {
                target: FeatureId(reader.u64()?),
                sketch: FeatureId(reader.u64()?),
                region: SketchRegionId(reader.u64()?),
                direction: if capabilities.feature_extents {
                    read_feature_direction(reader)?
                } else {
                    match reader.u8()? {
                        1 => FeatureDirection::AlongNormal,
                        2 => FeatureDirection::OppositeNormal,
                        value => return Err(PersistenceError::InvalidFeatureKind(value)),
                    }
                },
                extent: if capabilities.feature_extents {
                    read_feature_extent(reader)?
                } else {
                    FeatureExtent::Blind(read_extent_dimension(reader)?)
                },
                support: Box::new(read_exact_reference(reader)?),
            }),
            3 if capabilities.through_cut => FeatureKind::ThroughCut {
                target: FeatureId(reader.u64()?),
                profile: FeatureId(reader.u64()?),
            },
            4 if capabilities.revolve => {
                let profile = FeatureId(reader.u64()?);
                let (axis_start_mm, axis_end_mm, angle_degrees) = if capabilities.general_revolve {
                    (
                        [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)],
                        [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)],
                        f64::from_bits(reader.u64()?),
                    )
                } else {
                    ([0.0, 0.0], [0.0, 1.0], 360.0)
                };
                FeatureKind::Revolve {
                    profile,
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                }
            }
            5 if capabilities.shell => {
                let target = FeatureId(reader.u64()?);
                let removed_faces = if capabilities.stable_subshape_roles {
                    let mut roles = Vec::new();
                    for _ in 0..reader.count_with_limit(64)? {
                        roles.push(
                            StableFaceRole::new(reader.string()?)
                                .map_err(|_| PersistenceError::InvalidStableSubshapeRole)?,
                        );
                    }
                    roles
                } else {
                    vec![
                        StableFaceRole::new(BOTTLE_SHELL_OPENING_FACE_ROLE)
                            .expect("built-in bottle face role is valid"),
                    ]
                };
                FeatureKind::Shell {
                    target,
                    removed_faces,
                    thickness: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                }
            }
            6 if capabilities.bottle_finish => FeatureKind::BottleProfileControl {
                profile: FeatureId(reader.u64()?),
                body_radius: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                body_height: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                shoulder_rise: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            7 if capabilities.bottle_finish => {
                let target = FeatureId(reader.u64()?);
                let edges = if capabilities.stable_subshape_roles {
                    let mut roles = Vec::new();
                    for _ in 0..reader.count_with_limit(64)? {
                        roles.push(
                            StableEdgeRole::new(reader.string()?)
                                .map_err(|_| PersistenceError::InvalidStableSubshapeRole)?,
                        );
                    }
                    roles
                } else {
                    vec![
                        StableEdgeRole::new(BOTTLE_SHOULDER_EDGE_ROLE)
                            .expect("built-in bottle edge role is valid"),
                    ]
                };
                FeatureKind::BottleEdgeFinish {
                    target,
                    edges,
                    kind: match reader.u8()? {
                        1 => BottleEdgeFinishKind::Fillet,
                        2 => BottleEdgeFinishKind::Chamfer,
                        value => return Err(PersistenceError::InvalidFeatureKind(value)),
                    },
                    amount: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                }
            }
            21 if capabilities.topological_feature_references => {
                let target = FeatureId(reader.u64()?);
                let mut removed_faces = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    removed_faces.push(read_topological_reference(reader)?);
                }
                FeatureKind::TopologyShell {
                    target,
                    removed_faces,
                    thickness: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                }
            }
            22 if capabilities.topological_feature_references => {
                let target = FeatureId(reader.u64()?);
                let mut edges = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    edges.push(read_topological_reference(reader)?);
                }
                FeatureKind::TopologyEdgeFinish {
                    target,
                    edges,
                    kind: match reader.u8()? {
                        1 => EdgeFinishKind::Fillet,
                        2 => EdgeFinishKind::Chamfer,
                        value => return Err(PersistenceError::InvalidFeatureKind(value)),
                    },
                    amount: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                }
            }
            23 if capabilities.topological_feature_references => FeatureKind::TopologyFaceOffset {
                target: FeatureId(reader.u64()?),
                face: read_topological_reference(reader)?,
                distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            8 if capabilities.boolean => FeatureKind::Boolean {
                operation: match reader.u8()? {
                    1 => BooleanOperation::Cut,
                    2 => BooleanOperation::Union,
                    3 if capabilities.boolean_intersect => BooleanOperation::Intersect,
                    4 if capabilities.boolean_split => BooleanOperation::Split,
                    value => return Err(PersistenceError::InvalidFeatureKind(value)),
                },
                target: FeatureId(reader.u64()?),
                tool: FeatureId(reader.u64()?),
            },
            10 if capabilities.pocket => FeatureKind::Pocket {
                target: FeatureId(reader.u64()?),
                profile: FeatureId(reader.u64()?),
                depth: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            12 if capabilities.planar_offset => FeatureKind::PlanarOffset {
                profile: FeatureId(reader.u64()?),
                distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            13 if capabilities.sweep => FeatureKind::Sweep {
                profile: FeatureId(reader.u64()?),
                path: FeatureId(reader.u64()?),
            },
            15 if capabilities.loft_spline => {
                let mut sections = Vec::new();
                for _ in 0..reader.count_with_limit(16)? {
                    sections.push(LoftSection {
                        profile: FeatureId(reader.u64()?),
                        elevation_mm: f64::from_bits(reader.u64()?),
                    });
                }
                FeatureKind::Loft { sections }
            }
            24 if capabilities.rigid_transform_feature => {
                let target = FeatureId(reader.u64()?);
                let mut matrix = [0.0; 16];
                for value in &mut matrix {
                    *value = f64::from_bits(reader.u64()?);
                }
                FeatureKind::RigidTransform {
                    target,
                    transform: Transform::from_matrix(matrix)?,
                }
            }
            16 if capabilities.imported_exact_body => {
                let schema = reader.string()?;
                let import_id = ImportId(reader.u64()?);
                let source_sha256 = reader
                    .take(32)?
                    .try_into()
                    .map_err(|_| PersistenceError::Truncated)?;
                let source_byte_len = reader.u64()?;
                let result_fingerprint = reader.string()?;
                let solid_count = reader.u32()?;
                let topology_counts = if capabilities.imported_topology_counts {
                    match reader.u8()? {
                        0 => None,
                        1 => {
                            let mut counts = [0_u32; 5];
                            for count in &mut counts {
                                *count = reader.u32()?;
                            }
                            Some(counts)
                        }
                        value => return Err(PersistenceError::InvalidBoolean(value)),
                    }
                } else {
                    None
                };
                let volume_mm3 = f64::from_bits(reader.u64()?);
                let mut bounds_mm = [[0.0; 3]; 2];
                for coordinate in bounds_mm.iter_mut().flatten() {
                    *coordinate = f64::from_bits(reader.u64()?);
                }
                FeatureKind::ImportedExactBody(ImportedExactBodySpec {
                    schema,
                    import_id,
                    source_sha256,
                    source_byte_len,
                    result_fingerprint,
                    solid_count,
                    topology_counts,
                    volume_mm3,
                    bounds_mm,
                    backend: reader.string()?,
                    tolerance: reader.string()?,
                })
            }
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
                    3 => MeshAuthority::ImportedStl {
                        import_id: crate::import::ImportId(reader.u64()?),
                    },
                    4 if capabilities.sketchup_scene => MeshAuthority::ImportedSketchupScene {
                        import_id: crate::import::ImportId(reader.u64()?),
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
                let dimension =
                    read_persistent_dimension(reader, capabilities.general_parameter_paths)?;
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
        if capabilities.import_receipts {
            for _ in 0..reader.count_with_limit(MAX_IMPORT_OUTPUTS as u32)? {
                let receipt = read_import_receipt(reader, capabilities.sketchup_scene)?;
                let id = receipt.id();
                if product
                    .import_receipts
                    .insert(id, Arc::new(receipt))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateImport(id));
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
        if capabilities.assembly_contract {
            for _ in 0..reader.count()? {
                let id = OccurrenceId(reader.u64()?);
                if !product.grounded_occurrences.insert(id) {
                    return Err(PersistenceError::DuplicateGroundedOccurrence(id));
                }
            }
            for _ in 0..reader.count()? {
                let mate = read_assembly_mate(reader)?;
                if product
                    .assembly_mates
                    .insert(mate.id(), Arc::new(mate))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateAssemblyMate);
                }
            }
        }
        if capabilities.orthographic_drawing && !reader.is_finished() {
            for _ in 0..reader.count()? {
                let sheet = read_drawing_sheet(reader)?;
                let id = sheet.id();
                if product.drawing_sheets.insert(id, Arc::new(sheet)).is_some() {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::DrawingSheetAlreadyExists(id),
                    ));
                }
            }
        }
        if capabilities.body_contract {
            let mut seen_definitions = BTreeSet::new();
            for _ in 0..reader.count()? {
                let definition_id = DefinitionId(reader.u64()?);
                if !seen_definitions.insert(definition_id) {
                    return Err(PersistenceError::DuplicateDefinition(definition_id));
                }
                let existing = product.definitions.get(&definition_id).ok_or(
                    PersistenceError::InvalidCanonicalData(CanonicalError::DefinitionNotFound(
                        definition_id,
                    )),
                )?;
                let mut bodies = BTreeMap::new();
                for _ in 0..reader.count()? {
                    let body_id = BodyId(reader.u64()?);
                    let body = Body {
                        id: body_id,
                        name: reader.string()?,
                        visible: reader.boolean()?,
                        consumed_by: capabilities
                            .body_consumption
                            .then(|| reader.optional_id().map(|id| id.map(FeatureId)))
                            .transpose()?
                            .flatten(),
                    };
                    if bodies.insert(body_id, body).is_some() {
                        return Err(PersistenceError::InvalidCanonicalData(
                            CanonicalError::BodyAlreadyExists(definition_id, body_id),
                        ));
                    }
                }
                let active_body_id = BodyId(reader.u64()?);
                let mut feature_body_ownership = BTreeMap::new();
                for _ in 0..reader.count()? {
                    let feature_id = FeatureId(reader.u64()?);
                    let input_body_ids = read_ids(reader)?.into_iter().map(BodyId).collect();
                    let output_body_id = reader.optional_id()?.map(BodyId);
                    let ownership = FeatureBodyOwnership::new(input_body_ids, output_body_id)?;
                    if feature_body_ownership
                        .insert(feature_id, ownership)
                        .is_some()
                    {
                        return Err(PersistenceError::InvalidCanonicalData(
                            CanonicalError::InvalidBodyOwnership(feature_id),
                        ));
                    }
                }
                product.definitions.insert(
                    definition_id,
                    Arc::new(Definition {
                        bodies,
                        active_body_id,
                        feature_body_ownership,
                        ..existing.as_ref().clone()
                    }),
                );
            }
            if seen_definitions.len() != product.definitions.len() {
                return Err(PersistenceError::InvalidCanonicalData(
                    CanonicalError::InvalidBodyContract,
                ));
            }
        }
        if capabilities.body_feature_suppression {
            for _ in 0..reader.count()? {
                let definition_id = DefinitionId(reader.u64()?);
                let body_id = BodyId(reader.u64()?);
                let encoded_suppressed = read_ids(reader)?;
                let suppressed = encoded_suppressed
                    .iter()
                    .copied()
                    .map(FeatureId)
                    .collect::<BTreeSet<_>>();
                if suppressed.is_empty()
                    || suppressed.len() != encoded_suppressed.len()
                    || product
                        .body_feature_suppression
                        .insert((definition_id, body_id), suppressed)
                        .is_some()
                {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::InvalidFeatureSuppression(definition_id, body_id),
                    ));
                }
            }
        }
        if capabilities.classification_dimensions {
            for _ in 0..reader.count()? {
                let id = ClassificationDimensionId(reader.u64()?);
                let name = reader.string()?;
                let mut categories = BTreeMap::new();
                for _ in 0..reader.count()? {
                    let category_id = ClassificationCategoryId(reader.u64()?);
                    let category = ClassificationCategory {
                        id: category_id,
                        name: reader.string()?,
                    };
                    if categories.insert(category_id, category).is_some() {
                        return Err(PersistenceError::InvalidCanonicalData(
                            CanonicalError::InvalidClassificationDimension(id),
                        ));
                    }
                }
                let dimension = ClassificationDimension {
                    id,
                    name,
                    categories,
                };
                if product
                    .classification_dimensions
                    .insert(id, Arc::new(dimension))
                    .is_some()
                {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::InvalidClassificationDimension(id),
                    ));
                }
            }
            for _ in 0..reader.count()? {
                let occurrence_id = OccurrenceId(reader.u64()?);
                let dimension_id = ClassificationDimensionId(reader.u64()?);
                let category_id = ClassificationCategoryId(reader.u64()?);
                let dimension = product.classification_dimensions.get(&dimension_id).ok_or(
                    PersistenceError::InvalidCanonicalData(
                        CanonicalError::ClassificationDimensionNotFound(dimension_id),
                    ),
                )?;
                if !product.occurrences.contains_key(&occurrence_id) {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::OccurrenceNotFound(occurrence_id),
                    ));
                }
                if !dimension.categories.contains_key(&category_id) {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::ClassificationCategoryNotFound(dimension_id, category_id),
                    ));
                }
                if product
                    .classification_assignments
                    .insert((occurrence_id, dimension_id), category_id)
                    .is_some()
                {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::InvalidClassificationDimension(dimension_id),
                    ));
                }
            }
        }
        if capabilities.assembly_kinematics {
            for _ in 0..reader.count()? {
                let joint = read_assembly_joint(reader)?;
                if product
                    .assembly_joints
                    .insert(joint.id(), Arc::new(joint))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateAssemblyJoint);
                }
            }
            if capabilities.assembly_motion_couplings {
                for _ in 0..reader.count()? {
                    let coupling = read_assembly_motion_coupling(reader)?;
                    if product
                        .assembly_motion_couplings
                        .insert(coupling.id(), Arc::new(coupling))
                        .is_some()
                    {
                        return Err(PersistenceError::DuplicateAssemblyMotionCoupling);
                    }
                }
            }
            for _ in 0..reader.count()? {
                let study = read_assembly_motion_study(reader)?;
                if product
                    .assembly_motion_studies
                    .insert(study.id(), Arc::new(study))
                    .is_some()
                {
                    return Err(PersistenceError::DuplicateAssemblyMotionStudy);
                }
            }
            if capabilities.mechanical_contract {
                for _ in 0..reader.count()? {
                    let interface = read_mechanical_interface(reader)?;
                    if product
                        .mechanical_interfaces
                        .insert(interface.id(), Arc::new(interface))
                        .is_some()
                    {
                        return Err(PersistenceError::DuplicateMechanicalInterface);
                    }
                }
                for _ in 0..reader.count()? {
                    let condition = read_mechanical_condition(reader)?;
                    if product
                        .mechanical_conditions
                        .insert(condition.id(), Arc::new(condition))
                        .is_some()
                    {
                        return Err(PersistenceError::DuplicateMechanicalCondition);
                    }
                }
            }
        }
    }
    if !capabilities.body_contract {
        crate::document::migrate_legacy_body_contract(&mut product)?;
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
    InvalidImportFormat(u8),
    InvalidImportUnit(u8),
    InvalidImportDiagnostic(u8),
    InvalidImportOutput(u8),
    InvalidStableSubshapeRole,
    InvalidNodeKind(u8),
    InvalidPortType,
    InvalidOverrideMergePolicy,
    InvalidResolution(u8),
    InvalidReferenceStability(u8),
    InvalidOptionalMarker(u8),
    InvalidParameterSlot(u8),
    InvalidParameterPath,
    InvalidParameterValueType(u8),
    InvalidPersistentDimensionTarget(u8),
    InvalidDimensionDisplayUnit(u8),
    InvalidClearanceOwner(u8),
    InvalidClearanceCoordinateFrame,
    InvalidClearanceSeverity(u8),
    InvalidExactReference,
    InvalidAssemblyMate,
    InvalidAssemblyJoint,
    InvalidAssemblyMotionCoupling,
    InvalidAssemblyMotionStudy,
    InvalidMechanicalInterface,
    InvalidMechanicalCondition,
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
    DuplicateImport(ImportId),
    DuplicateNode(NodeId),
    DuplicateDefinition(DefinitionId),
    DuplicateFeature(FeatureId),
    DuplicateOccurrence(OccurrenceId),
    DuplicateGroundedOccurrence(OccurrenceId),
    DuplicateAssemblyMate,
    DuplicateAssemblyJoint,
    DuplicateAssemblyMotionCoupling,
    DuplicateAssemblyMotionStudy,
    DuplicateMechanicalInterface,
    DuplicateMechanicalCondition,
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
            Self::InvalidImportFormat(value) => {
                write!(formatter, "import format {value} is invalid")
            }
            Self::InvalidImportUnit(value) => write!(formatter, "import unit {value} is invalid"),
            Self::InvalidImportDiagnostic(value) => {
                write!(formatter, "import diagnostic severity {value} is invalid")
            }
            Self::InvalidImportOutput(value) => {
                write!(formatter, "import output kind {value} is invalid")
            }
            Self::InvalidStableSubshapeRole => {
                formatter.write_str("stable subshape role is invalid")
            }
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
            Self::InvalidParameterPath => formatter.write_str("feature parameter path is invalid"),
            Self::InvalidParameterValueType(value) => {
                write!(formatter, "feature parameter value type {value} is invalid")
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
            Self::InvalidAssemblyMate => formatter.write_str("assembly mate is invalid"),
            Self::InvalidAssemblyJoint => formatter.write_str("assembly joint is invalid"),
            Self::InvalidAssemblyMotionCoupling => {
                formatter.write_str("assembly motion coupling is invalid")
            }
            Self::InvalidMechanicalInterface => {
                formatter.write_str("mechanical interface is invalid")
            }
            Self::InvalidMechanicalCondition => {
                formatter.write_str("mechanical condition is invalid")
            }
            Self::InvalidAssemblyMotionStudy => {
                formatter.write_str("assembly motion study is invalid")
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
            Self::DuplicateImport(id) => {
                write!(formatter, "document repeats import {}", id.0)
            }
            Self::DuplicateNode(id) => write!(formatter, "document repeats node {}", id.0),
            Self::DuplicateDefinition(id) => {
                write!(formatter, "document repeats definition {}", id.0)
            }
            Self::DuplicateFeature(id) => write!(formatter, "document repeats feature {}", id.0),
            Self::DuplicateOccurrence(id) => {
                write!(formatter, "document repeats occurrence {}", id.0)
            }
            Self::DuplicateGroundedOccurrence(id) => {
                write!(formatter, "document repeats grounded occurrence {}", id.0)
            }
            Self::DuplicateAssemblyMate => formatter.write_str("document repeats an assembly mate"),
            Self::DuplicateAssemblyJoint => {
                formatter.write_str("document repeats an assembly joint")
            }
            Self::DuplicateAssemblyMotionCoupling => {
                formatter.write_str("document repeats an assembly motion coupling")
            }
            Self::DuplicateMechanicalInterface => {
                formatter.write_str("document repeats a mechanical interface")
            }
            Self::DuplicateMechanicalCondition => {
                formatter.write_str("document repeats a mechanical condition")
            }
            Self::DuplicateAssemblyMotionStudy => {
                formatter.write_str("document repeats an assembly motion study")
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
fn push_topological_reference(bytes: &mut Vec<u8>, reference: &TopologicalElementRef) {
    let encoded = reference
        .to_bytes()
        .expect("canonical topological feature reference is serializable");
    push_u32(bytes, encoded.len() as u32);
    bytes.extend_from_slice(&encoded);
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
