pub use crate::graph::{
    CanonicalOverride, DerivedIdentity, DerivedOutput, EvaluationIdentity, EvaluationReport,
    EvaluationStatus, EvaluatorNode, EvaluatorNodeKind, GraphError, OverrideMergePolicy,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment, ValueType,
};
use crate::graph::{
    evaluate_affected, evaluate_graph, resolve_derived_identity,
    validate_graph as validate_typed_graph,
};
use crate::prismatic::{CanonicalJoint, JointId, PrismaticError};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const COMMAND_SCHEMA_V1: &str = "ketchup.command.v1";
pub const TOLERANCE_PROFILE_V1: &str = "ketchup.tolerance.r0-v1";
const MAX_CANONICAL_ABS_MM: f64 = 1_000_000.0;

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);
    };
}

typed_id!(DocumentId);
typed_id!(DefinitionId);
typed_id!(OccurrenceId);
typed_id!(GroupId);
typed_id!(FeatureId);
typed_id!(TagId);
typed_id!(NodeId);
typed_id!(LocalOccurrenceId);
typed_id!(LocalGroupId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalOccurrenceKey {
    pub definition_id: DefinitionId,
    pub local_id: LocalOccurrenceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalGroupKey {
    pub definition_id: DefinitionId,
    pub local_id: LocalGroupId,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstancePath {
    root: OccurrenceId,
    steps: Vec<InstancePathStep>,
}

impl InstancePath {
    #[must_use]
    pub const fn root(root: OccurrenceId) -> Self {
        Self {
            root,
            steps: Vec::new(),
        }
    }

    #[must_use]
    pub const fn root_occurrence(&self) -> OccurrenceId {
        self.root
    }

    #[must_use]
    pub fn steps(&self) -> &[InstancePathStep] {
        &self.steps
    }

    #[must_use]
    pub fn with_step(&self, step: InstancePathStep) -> Self {
        let mut path = self.clone();
        path.steps.push(step);
        path
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.steps.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstancePathStep {
    Group(LocalGroupId),
    Occurrence(LocalOccurrenceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitSystem {
    Millimetres,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    matrix: [f64; 16],
}

impl Transform {
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn from_matrix(matrix: [f64; 16]) -> Result<Self, CanonicalError> {
        if matrix.iter().all(|value| value.is_finite())
            && matrix[12] == 0.0
            && matrix[13] == 0.0
            && matrix[14] == 0.0
            && matrix[15] == 1.0
        {
            Ok(Self { matrix })
        } else {
            Err(CanonicalError::InvalidTransform)
        }
    }

    pub fn from_translation(x_mm: f64, y_mm: f64, z_mm: f64) -> Result<Self, CanonicalError> {
        let mut matrix = Self::identity().matrix;
        matrix[3] = x_mm;
        matrix[7] = y_mm;
        matrix[11] = z_mm;
        Self::from_matrix(matrix)
    }

    #[must_use]
    pub const fn matrix(&self) -> &[f64; 16] {
        &self.matrix
    }

    #[must_use]
    pub fn compose(self, local: Self) -> Self {
        let mut result = [0.0; 16];
        for row in 0..4 {
            for column in 0..4 {
                result[row * 4 + column] = (0..4)
                    .map(|index| self.matrix[row * 4 + index] * local.matrix[index * 4 + column])
                    .sum();
            }
        }
        Self { matrix: result }
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureKind {
    Profile {
        points_mm: Vec<[f64; 2]>,
    },
    Extrusion {
        profile: FeatureId,
        height: Dimension,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub(crate) id: FeatureId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) kind: FeatureKind,
}

impl Feature {
    #[must_use]
    pub const fn id(&self) -> FeatureId {
        self.id
    }

    #[must_use]
    pub const fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> &FeatureKind {
        &self.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub(crate) id: DefinitionId,
    pub(crate) name: String,
    pub(crate) feature_ids: Vec<FeatureId>,
    pub(crate) local_occurrence_ids: Vec<LocalOccurrenceId>,
    pub(crate) local_group_ids: Vec<LocalGroupId>,
}

impl Definition {
    #[must_use]
    pub const fn id(&self) -> DefinitionId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn feature_ids(&self) -> &[FeatureId] {
        &self.feature_ids
    }

    #[must_use]
    pub fn local_occurrence_ids(&self) -> &[LocalOccurrenceId] {
        &self.local_occurrence_ids
    }

    #[must_use]
    pub fn local_group_ids(&self) -> &[LocalGroupId] {
        &self.local_group_ids
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub(crate) id: OccurrenceId,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<GroupId>,
    pub(crate) tag: Option<TagId>,
    pub(crate) visible: bool,
}

impl Occurrence {
    #[must_use]
    pub const fn id(&self) -> OccurrenceId {
        self.id
    }

    #[must_use]
    pub const fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<GroupId> {
        self.parent
    }

    #[must_use]
    pub const fn tag(&self) -> Option<TagId> {
        self.tag
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub(crate) id: GroupId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<GroupId>,
}

impl Group {
    #[must_use]
    pub const fn id(&self) -> GroupId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<GroupId> {
        self.parent
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalOccurrence {
    pub(crate) key: LocalOccurrenceKey,
    pub(crate) definition_id: DefinitionId,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<LocalGroupId>,
    pub(crate) tag: Option<TagId>,
    pub(crate) visible: bool,
}

impl LocalOccurrence {
    #[must_use]
    pub const fn key(&self) -> LocalOccurrenceKey {
        self.key
    }

    #[must_use]
    pub const fn definition_id(&self) -> DefinitionId {
        self.definition_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<LocalGroupId> {
        self.parent
    }

    #[must_use]
    pub const fn tag(&self) -> Option<TagId> {
        self.tag
    }

    #[must_use]
    pub const fn visible(&self) -> bool {
        self.visible
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalGroup {
    pub(crate) key: LocalGroupKey,
    pub(crate) name: String,
    pub(crate) transform: Transform,
    pub(crate) parent: Option<LocalGroupId>,
}

impl LocalGroup {
    #[must_use]
    pub const fn key(&self) -> LocalGroupKey {
        self.key
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    #[must_use]
    pub const fn parent(&self) -> Option<LocalGroupId> {
        self.parent
    }
}

#[derive(Clone)]
pub(crate) struct ProductModel {
    pub(crate) document_id: DocumentId,
    pub(crate) units: UnitSystem,
    pub(crate) evaluator_nodes: BTreeMap<NodeId, Arc<EvaluatorNode>>,
    pub(crate) overrides: BTreeMap<u64, Arc<CanonicalOverride>>,
    pub(crate) joints: BTreeMap<JointId, Arc<CanonicalJoint>>,
    pub(crate) definitions: BTreeMap<DefinitionId, Arc<Definition>>,
    pub(crate) features: BTreeMap<FeatureId, Arc<Feature>>,
    pub(crate) occurrences: BTreeMap<OccurrenceId, Arc<Occurrence>>,
    pub(crate) groups: BTreeMap<GroupId, Arc<Group>>,
    pub(crate) local_occurrences: BTreeMap<LocalOccurrenceKey, Arc<LocalOccurrence>>,
    pub(crate) local_groups: BTreeMap<LocalGroupKey, Arc<LocalGroup>>,
}

impl Default for ProductModel {
    fn default() -> Self {
        Self {
            document_id: allocate_document_id(),
            units: UnitSystem::Millimetres,
            evaluator_nodes: BTreeMap::new(),
            overrides: BTreeMap::new(),
            joints: BTreeMap::new(),
            definitions: BTreeMap::new(),
            features: BTreeMap::new(),
            occurrences: BTreeMap::new(),
            groups: BTreeMap::new(),
            local_occurrences: BTreeMap::new(),
            local_groups: BTreeMap::new(),
        }
    }
}

fn allocate_document_id() -> DocumentId {
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |duration| (duration.as_nanos() as u64).max(1));
    let value = NEXT
        .get_or_init(|| AtomicU64::new(seed))
        .fetch_add(1, Ordering::Relaxed);
    DocumentId(value.max(1))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneOccurrence {
    pub occurrence_id: OccurrenceId,
    pub instance_path: InstancePath,
    pub definition_id: DefinitionId,
    pub occurrence_name: String,
    pub definition_name: String,
    pub transform: Transform,
    pub parent: Option<GroupId>,
    pub local_parent: Option<LocalGroupId>,
    pub visible: bool,
    pub shared_occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    source_token: String,
    millimetres: f64,
}

impl Dimension {
    pub fn new(source_token: impl Into<String>, millimetres: f64) -> Result<Self, CanonicalError> {
        let source_token = source_token.into();
        if source_token.trim().is_empty() {
            return Err(CanonicalError::EmptySourceToken);
        }
        if !millimetres.is_finite() || millimetres.abs() > MAX_CANONICAL_ABS_MM {
            return Err(CanonicalError::DimensionOutsideEnvelope);
        }
        Ok(Self {
            source_token,
            millimetres,
        })
    }

    pub fn from_decimal(source_token: impl Into<String>) -> Result<Self, CanonicalError> {
        let source_token = source_token.into();
        let millimetres = source_token
            .parse::<f64>()
            .map_err(|_| CanonicalError::InvalidDecimalToken)?;
        Self::new(source_token, millimetres)
    }

    #[must_use]
    pub fn source_token(&self) -> &str {
        &self.source_token
    }

    #[must_use]
    pub const fn millimetres(&self) -> f64 {
        self.millimetres
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalCommand {
    CreateEvaluatorNode {
        id: NodeId,
        name: String,
        dimension: Dimension,
        dependencies: Vec<NodeId>,
    },
    SetEvaluatorDimension {
        id: NodeId,
        dimension: Dimension,
    },
    RenameEvaluatorNode {
        id: NodeId,
        name: String,
    },
    CreateExpressionNode {
        id: NodeId,
        name: String,
        expression: String,
    },
    CreateRuleNode {
        id: NodeId,
        name: String,
        expression: String,
        input_ports: Vec<PortSpec>,
        output_ports: Vec<PortSpec>,
        outputs: Vec<RuleOutput>,
        override_parameters: Vec<OverrideParameterSpec>,
    },
    SetNodeExpression {
        id: NodeId,
        expression: String,
    },
    SetRuleOutputs {
        id: NodeId,
        outputs: Vec<RuleOutput>,
    },
    UpsertOverride(CanonicalOverride),
    DeleteOverride {
        id: u64,
    },
    UpsertJoint(CanonicalJoint),
    DeleteJoint {
        id: JointId,
    },
    CreateDefinition {
        id: DefinitionId,
        name: String,
    },
    DeleteDefinition {
        id: DefinitionId,
    },
    RenameDefinition {
        id: DefinitionId,
        name: String,
    },
    CreateFeature {
        id: FeatureId,
        definition_id: DefinitionId,
        name: String,
        kind: FeatureKind,
    },
    DeleteFeature {
        id: FeatureId,
    },
    SetFeatureDimension {
        id: FeatureId,
        dimension: Dimension,
    },
    SetProfilePoints {
        id: FeatureId,
        points_mm: Vec<[f64; 2]>,
    },
    CreateOccurrence {
        id: OccurrenceId,
        definition_id: DefinitionId,
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
        tag: Option<TagId>,
        visible: bool,
    },
    DeleteOccurrence {
        id: OccurrenceId,
    },
    SetOccurrenceTransform {
        id: OccurrenceId,
        transform: Transform,
    },
    SetOccurrenceVisibility {
        id: OccurrenceId,
        visible: bool,
    },
    RepointOccurrence {
        id: OccurrenceId,
        definition_id: DefinitionId,
    },
    SetOccurrenceParent {
        id: OccurrenceId,
        parent: Option<GroupId>,
    },
    CreateGroup {
        id: GroupId,
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
    },
    DeleteGroup {
        id: GroupId,
    },
    SetGroupTransform {
        id: GroupId,
        transform: Transform,
    },
    SetGroupParent {
        id: GroupId,
        parent: Option<GroupId>,
    },
    CloneDefinitionAndRepoint(CloneDefinitionPlan),
    ConvertGroupToComponent(ConvertGroupPlan),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CloneDefinitionPlan {
    occurrence_id: OccurrenceId,
    source_definition_id: DefinitionId,
    new_definition_id: DefinitionId,
    new_definition_name: String,
    feature_id_map: Vec<(FeatureId, FeatureId)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvertGroupPlan {
    group_id: GroupId,
    new_definition_id: DefinitionId,
    new_occurrence_id: OccurrenceId,
    component_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthoritativeDependency {
    EvaluatorNode(NodeId),
    Override(u64),
    Joint(JointId),
    Definition(DefinitionId),
    Feature(FeatureId),
    Occurrence(OccurrenceId),
    Group(GroupId),
    LocalGroup(LocalGroupKey),
    LocalOccurrence(LocalOccurrenceKey),
    DefinitionUsers(DefinitionId),
    FeatureUsers(FeatureId),
    GroupChildren(GroupId),
    GroupSubtree(GroupId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorldEntityId {
    Group(GroupId),
    Occurrence(OccurrenceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertedEntityId {
    ComponentOccurrence(OccurrenceId),
    LocalGroup(LocalGroupKey),
    LocalOccurrence(LocalOccurrenceKey),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorldEntityPath {
    pub groups: Vec<GroupId>,
    pub occurrence: Option<OccurrenceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnresolvedMappingReason {
    NotInConvertedGroup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MappingResolution {
    Resolved {
        new_id: ConvertedEntityId,
        new_path: InstancePath,
    },
    Unresolved {
        reason: UnresolvedMappingReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionMapping {
    pub old_id: WorldEntityId,
    pub old_path: WorldEntityPath,
    pub resolution: MappingResolution,
}

pub struct ConvertGroupToComponentResult {
    pub revision: Arc<Revision>,
    pub component_definition_id: DefinitionId,
    pub component_occurrence_id: OccurrenceId,
    pub mappings: Vec<ConversionMapping>,
}

impl ConvertGroupToComponentResult {
    #[must_use]
    pub fn resolve_old_path(&self, old_path: &WorldEntityPath) -> MappingResolution {
        self.mappings
            .iter()
            .find(|mapping| &mapping.old_path == old_path)
            .map_or(
                MappingResolution::Unresolved {
                    reason: UnresolvedMappingReason::NotInConvertedGroup,
                },
                |mapping| mapping.resolution.clone(),
            )
    }

    pub fn unresolved_mappings(&self) -> impl Iterator<Item = &ConversionMapping> {
        self.mappings
            .iter()
            .filter(|mapping| matches!(mapping.resolution, MappingResolution::Unresolved { .. }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandBatch {
    schema: &'static str,
    commands: Vec<CanonicalCommand>,
}

impl CommandBatch {
    #[must_use]
    pub fn new(commands: Vec<CanonicalCommand>) -> Self {
        Self {
            schema: COMMAND_SCHEMA_V1,
            commands,
        }
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        self.schema
    }

    #[must_use]
    pub fn commands(&self) -> &[CanonicalCommand] {
        &self.commands
    }

    #[must_use]
    pub fn digest(&self) -> String {
        let mut digest = StableDigest::new();
        digest.bytes(self.schema.as_bytes());
        digest.u64(self.commands.len() as u64);
        for command in &self.commands {
            digest.command(command);
        }
        digest.finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedInstance {
    pub definition_id: DefinitionId,
    pub world_transform: Transform,
}

#[derive(Clone)]
pub struct Snapshot {
    revision_id: u64,
    product: Arc<ProductModel>,
}

impl Snapshot {
    #[must_use]
    pub const fn revision_id(&self) -> u64 {
        self.revision_id
    }

    #[must_use]
    pub fn evaluator_node(&self, id: NodeId) -> Option<&EvaluatorNode> {
        self.product.evaluator_nodes.get(&id).map(Arc::as_ref)
    }

    pub fn evaluator_nodes(&self) -> impl Iterator<Item = &EvaluatorNode> {
        self.product.evaluator_nodes.values().map(Arc::as_ref)
    }

    pub fn evaluator_node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.product.evaluator_nodes.keys().copied()
    }

    #[must_use]
    pub fn evaluator_node_count(&self) -> usize {
        self.product.evaluator_nodes.len()
    }

    #[must_use]
    pub fn shares_evaluator_node_with(&self, other: &Self, id: NodeId) -> bool {
        match (
            self.product.evaluator_nodes.get(&id),
            other.product.evaluator_nodes.get(&id),
        ) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn overrides(&self) -> impl Iterator<Item = &CanonicalOverride> {
        self.product.overrides.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn override_by_id(&self, id: u64) -> Option<&CanonicalOverride> {
        self.product.overrides.get(&id).map(Arc::as_ref)
    }

    pub fn joints(&self) -> impl Iterator<Item = &CanonicalJoint> {
        self.product.joints.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn joint(&self, id: JointId) -> Option<&CanonicalJoint> {
        self.product.joints.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn resolve_slot(&self, identity: &DerivedIdentity) -> SlotResolution {
        resolve_derived_identity(&self.product.evaluator_nodes, identity)
    }

    pub fn evaluate(
        &self,
        identity: &EvaluationIdentity,
    ) -> Result<EvaluationReport, CanonicalError> {
        let mut report = evaluate_graph(&self.product.evaluator_nodes, identity)
            .map_err(CanonicalError::Graph)?;
        report.document_id = Some(self.document_id());
        report.revision_id = Some(self.revision_id());
        report.canonical_digest = Some(self.canonical_digest());
        Ok(report)
    }

    #[must_use]
    pub fn canonical_digest(&self) -> String {
        digest_snapshot(self)
    }

    #[must_use]
    pub fn document_id(&self) -> DocumentId {
        self.product.document_id
    }

    #[must_use]
    pub fn units(&self) -> UnitSystem {
        self.product.units
    }

    #[must_use]
    pub fn definition(&self, id: DefinitionId) -> Option<&Definition> {
        self.product.definitions.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.product.features.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn occurrence(&self, id: OccurrenceId) -> Option<&Occurrence> {
        self.product.occurrences.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.product.groups.get(&id).map(Arc::as_ref)
    }

    pub fn definitions(&self) -> impl Iterator<Item = &Definition> {
        self.product.definitions.values().map(Arc::as_ref)
    }

    pub fn features(&self) -> impl Iterator<Item = &Feature> {
        self.product.features.values().map(Arc::as_ref)
    }

    pub fn occurrences(&self) -> impl Iterator<Item = &Occurrence> {
        self.product.occurrences.values().map(Arc::as_ref)
    }

    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.product.groups.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn local_occurrence(&self, key: LocalOccurrenceKey) -> Option<&LocalOccurrence> {
        self.product.local_occurrences.get(&key).map(Arc::as_ref)
    }

    #[must_use]
    pub fn local_group(&self, key: LocalGroupKey) -> Option<&LocalGroup> {
        self.product.local_groups.get(&key).map(Arc::as_ref)
    }

    pub fn local_occurrences(&self) -> impl Iterator<Item = &LocalOccurrence> {
        self.product.local_occurrences.values().map(Arc::as_ref)
    }

    pub fn local_groups(&self) -> impl Iterator<Item = &LocalGroup> {
        self.product.local_groups.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn scene_query(&self) -> Vec<SceneOccurrence> {
        let mut occurrences = Vec::new();
        for occurrence in self.product.occurrences.values() {
            let definition = &self.product.definitions[&occurrence.definition_id];
            let instance_path = InstancePath::root(occurrence.id);
            let world_transform = self
                .world_transform_for_occurrence(occurrence.id)
                .expect("validated occurrence hierarchy has a world transform");
            occurrences.push(SceneOccurrence {
                occurrence_id: occurrence.id,
                instance_path: instance_path.clone(),
                definition_id: definition.id,
                occurrence_name: occurrence.name.clone(),
                definition_name: definition.name.clone(),
                transform: world_transform,
                parent: occurrence.parent,
                local_parent: None,
                visible: occurrence.visible,
                shared_occurrence_count: 0,
            });
            project_local_occurrences(
                &self.product,
                occurrence.id,
                definition.id,
                &instance_path,
                world_transform,
                occurrence.visible,
                &mut occurrences,
            );
        }

        let mut sharing = BTreeMap::<DefinitionId, usize>::new();
        for occurrence in &occurrences {
            *sharing.entry(occurrence.definition_id).or_default() += 1;
        }
        for occurrence in &mut occurrences {
            occurrence.shared_occurrence_count = sharing[&occurrence.definition_id];
        }
        occurrences
    }

    pub fn resolve_instance_path(
        &self,
        path: &InstancePath,
    ) -> Result<ResolvedInstance, CanonicalError> {
        let root = self
            .occurrence(path.root)
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let mut definition_id = root.definition_id;
        let mut transform = self
            .world_transform_for_occurrence(root.id)
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let mut parent = None;
        for step in &path.steps {
            match *step {
                InstancePathStep::Group(local_id) => {
                    let group = self
                        .local_group(LocalGroupKey {
                            definition_id,
                            local_id,
                        })
                        .ok_or(CanonicalError::InvalidInstancePath)?;
                    if group.parent != parent {
                        return Err(CanonicalError::InvalidInstancePath);
                    }
                    transform = transform.compose(group.transform);
                    parent = Some(local_id);
                }
                InstancePathStep::Occurrence(local_id) => {
                    let occurrence = self
                        .local_occurrence(LocalOccurrenceKey {
                            definition_id,
                            local_id,
                        })
                        .ok_or(CanonicalError::InvalidInstancePath)?;
                    if occurrence.parent != parent {
                        return Err(CanonicalError::InvalidInstancePath);
                    }
                    transform = transform.compose(occurrence.transform);
                    definition_id = occurrence.definition_id;
                    parent = None;
                }
            }
        }
        Ok(ResolvedInstance {
            definition_id,
            world_transform: transform,
        })
    }

    #[must_use]
    pub fn world_transform_for_group(&self, id: GroupId) -> Option<Transform> {
        let mut lineage = Vec::new();
        let mut cursor = Some(id);
        while let Some(group_id) = cursor {
            let group = self.group(group_id)?;
            lineage.push(group_id);
            cursor = group.parent;
        }
        lineage.reverse();
        Some(
            lineage
                .into_iter()
                .fold(Transform::identity(), |transform, group_id| {
                    transform.compose(self.product.groups[&group_id].transform)
                }),
        )
    }

    #[must_use]
    pub fn world_transform_for_occurrence(&self, id: OccurrenceId) -> Option<Transform> {
        let occurrence = self.occurrence(id)?;
        let parent_transform = occurrence
            .parent
            .map_or(Some(Transform::identity()), |parent| {
                self.world_transform_for_group(parent)
            })?;
        Some(parent_transform.compose(occurrence.transform))
    }

    pub(crate) fn product(&self) -> &ProductModel {
        &self.product
    }
}

fn project_local_occurrences(
    product: &ProductModel,
    root_occurrence_id: OccurrenceId,
    owner_definition_id: DefinitionId,
    owner_path: &InstancePath,
    owner_transform: Transform,
    owner_visible: bool,
    output: &mut Vec<SceneOccurrence>,
) {
    let definition = &product.definitions[&owner_definition_id];
    for local_id in &definition.local_occurrence_ids {
        let local = &product.local_occurrences[&LocalOccurrenceKey {
            definition_id: owner_definition_id,
            local_id: *local_id,
        }];
        let mut group_lineage = Vec::new();
        let mut parent = local.parent;
        while let Some(local_group_id) = parent {
            let group = &product.local_groups[&LocalGroupKey {
                definition_id: owner_definition_id,
                local_id: local_group_id,
            }];
            group_lineage.push(local_group_id);
            parent = group.parent;
        }
        group_lineage.reverse();

        let mut path = owner_path.clone();
        let mut world_transform = owner_transform;
        for local_group_id in group_lineage {
            let group = &product.local_groups[&LocalGroupKey {
                definition_id: owner_definition_id,
                local_id: local_group_id,
            }];
            path = path.with_step(InstancePathStep::Group(local_group_id));
            world_transform = world_transform.compose(group.transform);
        }
        path = path.with_step(InstancePathStep::Occurrence(*local_id));
        world_transform = world_transform.compose(local.transform);
        let target_definition = &product.definitions[&local.definition_id];
        let visible = owner_visible && local.visible;
        output.push(SceneOccurrence {
            occurrence_id: root_occurrence_id,
            instance_path: path.clone(),
            definition_id: local.definition_id,
            occurrence_name: local.name.clone(),
            definition_name: target_definition.name.clone(),
            transform: world_transform,
            parent: None,
            local_parent: local.parent,
            visible,
            shared_occurrence_count: 0,
        });
        project_local_occurrences(
            product,
            root_occurrence_id,
            local.definition_id,
            &path,
            world_transform,
            visible,
            output,
        );
    }
}

#[derive(Clone)]
pub struct Revision {
    id: u64,
    snapshot: Snapshot,
    batch_digest: String,
    recomputed_nodes: BTreeSet<NodeId>,
    evaluation: Option<EvaluationReport>,
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
    pub const fn recomputed_nodes(&self) -> &BTreeSet<NodeId> {
        &self.recomputed_nodes
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
pub struct DerivedResultEvent {
    pub key: DerivedResultKey,
    pub classification: DerivedResultClassification,
}

pub struct DocumentStore {
    revisions: Vec<Arc<Revision>>,
    cursor: usize,
    next_revision_id: u64,
    evaluation_registry: BTreeMap<DerivedResultKey, DerivedResultEvent>,
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::from_product(0, ProductModel::default())
            .expect("an empty canonical document is valid")
    }

    pub(crate) fn from_product(
        revision_id: u64,
        product: ProductModel,
    ) -> Result<Self, CanonicalError> {
        validate_graph(&product.evaluator_nodes)?;
        validate_overrides(&product)?;
        validate_product(&product)?;
        let next_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let snapshot = Snapshot {
            revision_id,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: String::new(),
            recomputed_nodes: BTreeSet::new(),
            evaluation: None,
        });
        Ok(Self {
            revisions: vec![revision],
            cursor: 0,
            next_revision_id,
            evaluation_registry: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn current(&self) -> Snapshot {
        self.revisions[self.cursor].snapshot.clone()
    }

    #[must_use]
    pub fn revision_count(&self) -> usize {
        self.revisions.len()
    }

    #[must_use]
    pub const fn visible_undo_steps(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn visible_redo_steps(&self) -> usize {
        self.revisions.len() - self.cursor - 1
    }

    pub fn discard_history_before_current(&mut self) {
        let current = Arc::clone(&self.revisions[self.cursor]);
        self.revisions.clear();
        self.revisions.push(current);
        self.cursor = 0;
    }

    pub fn apply_batch(&mut self, batch: &CommandBatch) -> Result<Arc<Revision>, CanonicalError> {
        if batch.schema != COMMAND_SCHEMA_V1 {
            return Err(CanonicalError::UnsupportedCommandSchema);
        }
        if batch.commands.is_empty() {
            return Err(CanonicalError::EmptyCommandBatch);
        }

        let current = self.current();
        let mut product = current.product.as_ref().clone();
        let mut changed_evaluator_nodes = BTreeSet::new();

        for command in &batch.commands {
            match command {
                CanonicalCommand::CreateEvaluatorNode {
                    id,
                    name,
                    dimension,
                    dependencies,
                } => {
                    if product.evaluator_nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    for dependency in dependencies {
                        if !product.evaluator_nodes.contains_key(dependency) {
                            return Err(CanonicalError::MissingDependency(*dependency));
                        }
                    }
                    let node = EvaluatorNode::parameter(
                        *id,
                        name.clone(),
                        dimension.clone(),
                        dependencies.clone(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(node));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::SetEvaluatorDimension { id, dimension } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = EvaluatorNode::parameter(
                        *id,
                        existing.name.clone(),
                        dimension.clone(),
                        existing.dependencies.clone(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::RenameEvaluatorNode { id, name } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = match &existing.kind {
                        EvaluatorNodeKind::Parameter { value } => EvaluatorNode::parameter(
                            *id,
                            name.clone(),
                            value.clone(),
                            existing.dependencies.clone(),
                        ),
                        EvaluatorNodeKind::Expression { source, .. } => {
                            EvaluatorNode::expression(*id, name.clone(), source.clone())
                        }
                        EvaluatorNodeKind::Rule {
                            source, outputs, ..
                        } => EvaluatorNode::rule(
                            *id,
                            name.clone(),
                            source.clone(),
                            existing.input_ports.clone(),
                            existing.output_ports.clone(),
                            outputs.clone(),
                            existing.allowed_parameters().to_vec(),
                        ),
                    }
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::CreateExpressionNode {
                    id,
                    name,
                    expression,
                } => {
                    if product.evaluator_nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    let node = EvaluatorNode::expression(*id, name.clone(), expression.clone())
                        .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(node));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::CreateRuleNode {
                    id,
                    name,
                    expression,
                    input_ports,
                    output_ports,
                    outputs,
                    override_parameters,
                } => {
                    if product.evaluator_nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    let node = EvaluatorNode::rule(
                        *id,
                        name.clone(),
                        expression.clone(),
                        input_ports.clone(),
                        output_ports.clone(),
                        outputs.clone(),
                        override_parameters.clone(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(node));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::SetNodeExpression { id, expression } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = match &existing.kind {
                        EvaluatorNodeKind::Expression { .. } => EvaluatorNode::expression(
                            *id,
                            existing.name.clone(),
                            expression.clone(),
                        ),
                        EvaluatorNodeKind::Rule { outputs, .. } => EvaluatorNode::rule(
                            *id,
                            existing.name.clone(),
                            expression.clone(),
                            existing.input_ports.clone(),
                            existing.output_ports.clone(),
                            outputs.clone(),
                            existing.allowed_parameters().to_vec(),
                        ),
                        EvaluatorNodeKind::Parameter { .. } => {
                            return Err(CanonicalError::WrongNodeKind(*id));
                        }
                    }
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::SetRuleOutputs { id, outputs } => {
                    let existing = product
                        .evaluator_nodes
                        .get(id)
                        .ok_or(CanonicalError::NodeNotFound(*id))?;
                    let EvaluatorNodeKind::Rule { source, .. } = &existing.kind else {
                        return Err(CanonicalError::WrongNodeKind(*id));
                    };
                    let replacement = EvaluatorNode::rule(
                        *id,
                        existing.name.clone(),
                        source.clone(),
                        existing.input_ports.clone(),
                        existing.output_ports.clone(),
                        outputs.clone(),
                        existing.allowed_parameters().to_vec(),
                    )
                    .map_err(CanonicalError::Graph)?;
                    product.evaluator_nodes.insert(*id, Arc::new(replacement));
                    changed_evaluator_nodes.insert(*id);
                }
                CanonicalCommand::UpsertOverride(spec) => {
                    let mut canonical = spec.clone();
                    canonical.health =
                        resolve_derived_identity(&product.evaluator_nodes, &canonical.target);
                    product.overrides.insert(canonical.id, Arc::new(canonical));
                }
                CanonicalCommand::DeleteOverride { id } => {
                    if product.overrides.remove(id).is_none() {
                        return Err(CanonicalError::OverrideNotFound(*id));
                    }
                }
                CanonicalCommand::UpsertJoint(joint) => {
                    product.joints.insert(joint.id(), Arc::new(joint.clone()));
                }
                CanonicalCommand::DeleteJoint { id } => {
                    if product.joints.remove(id).is_none() {
                        return Err(CanonicalError::JointNotFound(*id));
                    }
                }
                CanonicalCommand::CreateDefinition { id, name } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    if product.definitions.contains_key(id) {
                        return Err(CanonicalError::DefinitionAlreadyExists(*id));
                    }
                    product.definitions.insert(
                        *id,
                        Arc::new(Definition {
                            id: *id,
                            name: name.clone(),
                            feature_ids: Vec::new(),
                            local_occurrence_ids: Vec::new(),
                            local_group_ids: Vec::new(),
                        }),
                    );
                }
                CanonicalCommand::DeleteDefinition { id } => {
                    if product
                        .occurrences
                        .values()
                        .any(|occurrence| occurrence.definition_id == *id)
                    {
                        return Err(CanonicalError::DefinitionInUse(*id));
                    }
                    let definition = product
                        .definitions
                        .remove(id)
                        .ok_or(CanonicalError::DefinitionNotFound(*id))?;
                    for feature_id in &definition.feature_ids {
                        product.features.remove(feature_id);
                    }
                }
                CanonicalCommand::RenameDefinition { id, name } => {
                    ensure_name(name)?;
                    let existing = product
                        .definitions
                        .get(id)
                        .ok_or(CanonicalError::DefinitionNotFound(*id))?;
                    product.definitions.insert(
                        *id,
                        Arc::new(Definition {
                            id: *id,
                            name: name.clone(),
                            feature_ids: existing.feature_ids.clone(),
                            local_occurrence_ids: existing.local_occurrence_ids.clone(),
                            local_group_ids: existing.local_group_ids.clone(),
                        }),
                    );
                }
                CanonicalCommand::CreateFeature {
                    id,
                    definition_id,
                    name,
                    kind,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    validate_feature_kind(kind)?;
                    if product.features.contains_key(id) {
                        return Err(CanonicalError::FeatureAlreadyExists(*id));
                    }
                    let definition = product
                        .definitions
                        .get(definition_id)
                        .ok_or(CanonicalError::DefinitionNotFound(*definition_id))?;
                    let mut feature_ids = definition.feature_ids.clone();
                    feature_ids.push(*id);
                    product.definitions.insert(
                        *definition_id,
                        Arc::new(Definition {
                            id: *definition_id,
                            name: definition.name.clone(),
                            feature_ids,
                            local_occurrence_ids: definition.local_occurrence_ids.clone(),
                            local_group_ids: definition.local_group_ids.clone(),
                        }),
                    );
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: *definition_id,
                            name: name.clone(),
                            kind: kind.clone(),
                        }),
                    );
                }
                CanonicalCommand::DeleteFeature { id } => {
                    let feature = product
                        .features
                        .remove(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let definition = &product.definitions[&feature.definition_id];
                    let feature_ids = definition
                        .feature_ids
                        .iter()
                        .copied()
                        .filter(|candidate| candidate != id)
                        .collect();
                    product.definitions.insert(
                        feature.definition_id,
                        Arc::new(Definition {
                            id: definition.id,
                            name: definition.name.clone(),
                            feature_ids,
                            local_occurrence_ids: definition.local_occurrence_ids.clone(),
                            local_group_ids: definition.local_group_ids.clone(),
                        }),
                    );
                }
                CanonicalCommand::SetFeatureDimension { id, dimension } => {
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    let FeatureKind::Extrusion { profile, .. } = feature.kind else {
                        return Err(CanonicalError::FeatureHasNoDimension(*id));
                    };
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind: FeatureKind::Extrusion {
                                profile,
                                height: dimension.clone(),
                            },
                        }),
                    );
                }
                CanonicalCommand::SetProfilePoints { id, points_mm } => {
                    validate_feature_kind(&FeatureKind::Profile {
                        points_mm: points_mm.clone(),
                    })?;
                    let feature = product
                        .features
                        .get(id)
                        .ok_or(CanonicalError::FeatureNotFound(*id))?;
                    if !matches!(feature.kind, FeatureKind::Profile { .. }) {
                        return Err(CanonicalError::FeatureIsNotProfile(*id));
                    }
                    product.features.insert(
                        *id,
                        Arc::new(Feature {
                            id: *id,
                            definition_id: feature.definition_id,
                            name: feature.name.clone(),
                            kind: FeatureKind::Profile {
                                points_mm: points_mm.clone(),
                            },
                        }),
                    );
                }
                CanonicalCommand::CreateOccurrence {
                    id,
                    definition_id,
                    name,
                    transform,
                    parent,
                    tag,
                    visible,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    validate_transform(*transform)?;
                    if product.occurrences.contains_key(id) {
                        return Err(CanonicalError::OccurrenceAlreadyExists(*id));
                    }
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            id: *id,
                            definition_id: *definition_id,
                            name: name.clone(),
                            transform: *transform,
                            parent: *parent,
                            tag: *tag,
                            visible: *visible,
                        }),
                    );
                }
                CanonicalCommand::DeleteOccurrence { id } => {
                    product
                        .occurrences
                        .remove(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                }
                CanonicalCommand::SetOccurrenceTransform { id, transform } => {
                    validate_transform(*transform)?;
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            transform: *transform,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetOccurrenceVisibility { id, visible } => {
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            visible: *visible,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::RepointOccurrence { id, definition_id } => {
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            definition_id: *definition_id,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetOccurrenceParent { id, parent } => {
                    let existing = product
                        .occurrences
                        .get(id)
                        .ok_or(CanonicalError::OccurrenceNotFound(*id))?;
                    product.occurrences.insert(
                        *id,
                        Arc::new(Occurrence {
                            parent: *parent,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::CreateGroup {
                    id,
                    name,
                    transform,
                    parent,
                } => {
                    ensure_product_id(id.0)?;
                    ensure_name(name)?;
                    validate_transform(*transform)?;
                    if product.groups.contains_key(id) {
                        return Err(CanonicalError::GroupAlreadyExists(*id));
                    }
                    product.groups.insert(
                        *id,
                        Arc::new(Group {
                            id: *id,
                            name: name.clone(),
                            transform: *transform,
                            parent: *parent,
                        }),
                    );
                }
                CanonicalCommand::DeleteGroup { id } => {
                    if product
                        .occurrences
                        .values()
                        .any(|occurrence| occurrence.parent == Some(*id))
                        || product
                            .groups
                            .values()
                            .any(|group| group.parent == Some(*id))
                    {
                        return Err(CanonicalError::GroupNotEmpty(*id));
                    }
                    product
                        .groups
                        .remove(id)
                        .ok_or(CanonicalError::GroupNotFound(*id))?;
                }
                CanonicalCommand::SetGroupTransform { id, transform } => {
                    validate_transform(*transform)?;
                    let existing = product
                        .groups
                        .get(id)
                        .ok_or(CanonicalError::GroupNotFound(*id))?;
                    product.groups.insert(
                        *id,
                        Arc::new(Group {
                            transform: *transform,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::SetGroupParent { id, parent } => {
                    let existing = product
                        .groups
                        .get(id)
                        .ok_or(CanonicalError::GroupNotFound(*id))?;
                    product.groups.insert(
                        *id,
                        Arc::new(Group {
                            parent: *parent,
                            ..existing.as_ref().clone()
                        }),
                    );
                }
                CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                    clone_definition_and_repoint(&mut product, plan)?;
                }
                CanonicalCommand::ConvertGroupToComponent(plan) => {
                    convert_group_to_component_model(&mut product, plan)?;
                }
            }
        }

        validate_graph(&product.evaluator_nodes)?;
        refresh_override_health(&mut product);
        validate_overrides(&product)?;
        validate_product(&product)?;
        let revision_id = self.next_revision_id;
        let following_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let recomputed_nodes =
            dependent_closure(&product.evaluator_nodes, &changed_evaluator_nodes);
        let evaluation = evaluate_affected(
            &product.evaluator_nodes,
            &EvaluationIdentity::default(),
            self.revisions[self.cursor].evaluation.as_ref(),
            &recomputed_nodes,
        )
        .map_err(CanonicalError::Graph)?;
        let snapshot = Snapshot {
            revision_id,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: batch.digest(),
            recomputed_nodes,
            evaluation: Some(evaluation),
        });

        self.revisions.truncate(self.cursor + 1);
        self.revisions.push(Arc::clone(&revision));
        self.cursor += 1;
        self.next_revision_id = following_revision_id;
        Ok(revision)
    }

    pub fn make_unique(
        &mut self,
        occurrence_id: OccurrenceId,
        new_definition_name: impl Into<String>,
    ) -> Result<Arc<Revision>, CanonicalError> {
        let snapshot = self.current();
        let occurrence = snapshot
            .occurrence(occurrence_id)
            .ok_or(CanonicalError::OccurrenceNotFound(occurrence_id))?;
        let source = snapshot
            .definition(occurrence.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(occurrence.definition_id))?;
        let new_definition_id =
            DefinitionId(next_id(snapshot.definitions().map(|item| item.id.0))?);
        let mut next_feature_id = next_id(snapshot.features().map(|item| item.id.0))?;
        let feature_id_map = source
            .feature_ids
            .iter()
            .map(|source_id| {
                let mapped = FeatureId(next_feature_id);
                next_feature_id += 1;
                (*source_id, mapped)
            })
            .collect();
        let plan = CloneDefinitionPlan {
            occurrence_id,
            source_definition_id: occurrence.definition_id,
            new_definition_id,
            new_definition_name: new_definition_name.into(),
            feature_id_map,
        };
        self.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CloneDefinitionAndRepoint(plan),
        ]))
    }

    pub fn convert_group_to_component(
        &mut self,
        group_id: GroupId,
        component_name: impl Into<String>,
    ) -> Result<ConvertGroupToComponentResult, CanonicalError> {
        let snapshot = self.current();
        if snapshot.group(group_id).is_none() {
            return Err(CanonicalError::GroupNotFound(group_id));
        }
        let plan = ConvertGroupPlan {
            group_id,
            new_definition_id: DefinitionId(next_id(snapshot.definitions().map(|item| item.id.0))?),
            new_occurrence_id: OccurrenceId(next_id(snapshot.occurrences().map(|item| item.id.0))?),
            component_name: component_name.into(),
        };
        let mappings = conversion_mappings(snapshot.product(), &plan)?;
        let component_definition_id = plan.new_definition_id;
        let component_occurrence_id = plan.new_occurrence_id;
        let revision = self.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::ConvertGroupToComponent(plan),
        ]))?;
        Ok(ConvertGroupToComponentResult {
            revision,
            component_definition_id,
            component_occurrence_id,
            mappings,
        })
    }

    pub fn register_evaluation(
        &mut self,
        root_rule_node_id: NodeId,
        slot_path: SlotPath,
        report: &EvaluationReport,
    ) -> Result<DerivedResultEvent, CanonicalError> {
        let snapshot = self.current();
        let Some(root_node) = snapshot.evaluator_node(root_rule_node_id) else {
            return Err(CanonicalError::NodeNotFound(root_rule_node_id));
        };
        if !matches!(root_node.kind(), EvaluatorNodeKind::Rule { .. }) {
            return Err(CanonicalError::WrongNodeKind(root_rule_node_id));
        }
        let target = DerivedIdentity::new(root_rule_node_id, slot_path.clone())
            .map_err(CanonicalError::Graph)?;
        if resolve_derived_identity(&snapshot.product.evaluator_nodes, &target)
            != SlotResolution::Resolved
        {
            return Err(CanonicalError::UnresolvedDerivedOutput);
        }
        if report.document_id != Some(snapshot.document_id())
            || report.revision_id != Some(snapshot.revision_id())
            || report.canonical_digest.as_deref() != Some(snapshot.canonical_digest().as_str())
        {
            return Err(CanonicalError::EvaluationEnvelopeMismatch);
        }
        let expected = snapshot.evaluate(&report.identity)?;
        let supplied_node = report
            .node(root_rule_node_id)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        let expected_node = expected
            .node(root_rule_node_id)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        if supplied_node != expected_node {
            return Err(CanonicalError::EvaluationEvidenceMismatch);
        }
        if !matches!(supplied_node.status, EvaluationStatus::Evaluated(_)) {
            return Err(CanonicalError::FailedEvaluation(root_rule_node_id));
        }
        let output = report
            .outputs
            .get(&target)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        let expected_output = expected
            .outputs
            .get(&target)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        if output != expected_output {
            return Err(CanonicalError::EvaluationEvidenceMismatch);
        }
        let key = DerivedResultKey {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            root_rule_node_id,
            slot_path,
            input_digest: output.input_digest.clone(),
            result_digest: output.result_digest.clone(),
            evaluator: report.identity.evaluator.clone(),
            backend: report.identity.backend.clone(),
            schema: report.identity.schema.clone(),
            tolerance: report.identity.tolerance.clone(),
        };
        let event = DerivedResultEvent {
            key: key.clone(),
            classification: DerivedResultClassification::Current,
        };
        self.evaluation_registry.insert(key, event.clone());
        Ok(event)
    }

    #[must_use]
    pub fn evaluation_registry_len(&self) -> usize {
        self.evaluation_registry.len()
    }

    pub fn undo(&mut self) -> Option<Snapshot> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        Some(self.current())
    }

    pub fn redo(&mut self) -> Option<Snapshot> {
        if self.cursor + 1 >= self.revisions.len() {
            return None;
        }
        self.cursor += 1;
        Some(self.current())
    }

    #[must_use]
    pub fn prepare_proposal(&self, batch: CommandBatch) -> Proposal {
        let snapshot = self.current();
        let authoritative_dependencies = authoritative_dependencies(&snapshot, &batch);
        Proposal {
            provenance_revision: snapshot.revision_id,
            command_digest: batch.digest(),
            dependency_digest: dependency_digest(&snapshot, &authoritative_dependencies),
            authoritative_dependencies,
            batch,
        }
    }

    #[must_use]
    pub fn validate_proposal(&self, proposal: &Proposal) -> ProposalValidity {
        let snapshot = self.current();
        let command_matches = proposal.command_digest == proposal.batch.digest();
        let dependencies_match = proposal.dependency_digest
            == dependency_digest(&snapshot, &proposal.authoritative_dependencies);
        if command_matches && dependencies_match {
            ProposalValidity::Valid {
                evaluated_revision: snapshot.revision_id,
            }
        } else {
            ProposalValidity::Stale {
                provenance_revision: proposal.provenance_revision,
                current_revision: snapshot.revision_id,
            }
        }
    }

    pub fn commit_proposal(
        &mut self,
        proposal: &Proposal,
    ) -> Result<Arc<Revision>, ProposalCommitError> {
        match self.validate_proposal(proposal) {
            ProposalValidity::Valid { .. } => self
                .apply_batch(&proposal.batch)
                .map_err(ProposalCommitError::Canonical),
            stale @ ProposalValidity::Stale { .. } => Err(ProposalCommitError::Stale(stale)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    provenance_revision: u64,
    batch: CommandBatch,
    command_digest: String,
    authoritative_dependencies: BTreeSet<AuthoritativeDependency>,
    dependency_digest: String,
}

impl Proposal {
    #[must_use]
    pub const fn provenance_revision(&self) -> u64 {
        self.provenance_revision
    }

    #[must_use]
    pub const fn batch(&self) -> &CommandBatch {
        &self.batch
    }

    #[must_use]
    pub const fn authoritative_dependencies(&self) -> &BTreeSet<AuthoritativeDependency> {
        &self.authoritative_dependencies
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

#[derive(Debug, PartialEq)]
pub enum ProposalCommitError {
    Stale(ProposalValidity),
    Canonical(CanonicalError),
}

impl fmt::Display for ProposalCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale(_) => formatter.write_str("proposal dependencies changed"),
            Self::Canonical(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProposalCommitError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalError {
    EmptySourceToken,
    InvalidDecimalToken,
    DimensionOutsideEnvelope,
    ReservedNodeId,
    EmptyNodeName,
    DependenciesNotCanonical,
    DependencyCycle(NodeId),
    NodeAlreadyExists(NodeId),
    NodeNotFound(NodeId),
    MissingDependency(NodeId),
    UnsupportedCommandSchema,
    EmptyCommandBatch,
    ReservedProductId,
    EmptyProductName,
    InvalidTransform,
    InvalidProfile,
    DefinitionAlreadyExists(DefinitionId),
    DefinitionNotFound(DefinitionId),
    DefinitionInUse(DefinitionId),
    FeatureAlreadyExists(FeatureId),
    FeatureNotFound(FeatureId),
    FeatureHasNoDimension(FeatureId),
    FeatureIsNotProfile(FeatureId),
    OccurrenceAlreadyExists(OccurrenceId),
    OccurrenceNotFound(OccurrenceId),
    GroupAlreadyExists(GroupId),
    GroupNotFound(GroupId),
    GroupNotEmpty(GroupId),
    GroupCycle(GroupId),
    InvalidFeatureOwnership(FeatureId),
    InvalidFeatureMap,
    OccurrenceDefinitionMismatch,
    InvalidLocalGraph,
    InvalidInstancePath,
    IdExhausted,
    WrongNodeKind(NodeId),
    OverrideNotFound(u64),
    JointNotFound(JointId),
    UndeclaredOverrideParameter,
    UnresolvedDerivedOutput,
    EvaluationEnvelopeMismatch,
    EvaluationEvidenceMismatch,
    FailedEvaluation(NodeId),
    RevisionExhausted,
    Graph(GraphError),
    Prismatic(PrismaticError),
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySourceToken => formatter.write_str("dimension source token is empty"),
            Self::InvalidDecimalToken => {
                formatter.write_str("dimension source token is not decimal")
            }
            Self::DimensionOutsideEnvelope => {
                formatter.write_str("dimension is outside the canonical coordinate envelope")
            }
            Self::ReservedNodeId => formatter.write_str("node ID zero is reserved"),
            Self::EmptyNodeName => formatter.write_str("node name is empty"),
            Self::DependenciesNotCanonical => {
                formatter.write_str("dependencies must be unique and strictly sorted")
            }
            Self::DependencyCycle(id) => write!(formatter, "dependency cycle at node {}", id.0),
            Self::NodeAlreadyExists(id) => write!(formatter, "node {} already exists", id.0),
            Self::NodeNotFound(id) => write!(formatter, "node {} does not exist", id.0),
            Self::MissingDependency(id) => write!(formatter, "dependency {} does not exist", id.0),
            Self::UnsupportedCommandSchema => formatter.write_str("unsupported command schema"),
            Self::EmptyCommandBatch => formatter.write_str("command batch is empty"),
            Self::ReservedProductId => formatter.write_str("product entity ID zero is reserved"),
            Self::EmptyProductName => formatter.write_str("product entity name is empty"),
            Self::InvalidTransform => {
                formatter.write_str("transform is not a finite affine matrix")
            }
            Self::InvalidProfile => {
                formatter.write_str("profile must contain finite non-degenerate points")
            }
            Self::DefinitionAlreadyExists(id) => {
                write!(formatter, "definition {} already exists", id.0)
            }
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} does not exist", id.0),
            Self::DefinitionInUse(id) => write!(formatter, "definition {} is still used", id.0),
            Self::FeatureAlreadyExists(id) => write!(formatter, "feature {} already exists", id.0),
            Self::FeatureNotFound(id) => write!(formatter, "feature {} does not exist", id.0),
            Self::FeatureHasNoDimension(id) => {
                write!(formatter, "feature {} has no editable dimension", id.0)
            }
            Self::FeatureIsNotProfile(id) => {
                write!(formatter, "feature {} is not a profile", id.0)
            }
            Self::OccurrenceAlreadyExists(id) => {
                write!(formatter, "occurrence {} already exists", id.0)
            }
            Self::OccurrenceNotFound(id) => write!(formatter, "occurrence {} does not exist", id.0),
            Self::GroupAlreadyExists(id) => write!(formatter, "group {} already exists", id.0),
            Self::GroupNotFound(id) => write!(formatter, "group {} does not exist", id.0),
            Self::GroupNotEmpty(id) => write!(formatter, "group {} is not empty", id.0),
            Self::GroupCycle(id) => write!(formatter, "group hierarchy cycle at {}", id.0),
            Self::InvalidFeatureOwnership(id) => write!(
                formatter,
                "feature {} has invalid definition ownership",
                id.0
            ),
            Self::InvalidFeatureMap => {
                formatter.write_str("feature clone map is incomplete or non-canonical")
            }
            Self::OccurrenceDefinitionMismatch => {
                formatter.write_str("occurrence does not reference the requested source definition")
            }
            Self::InvalidLocalGraph => formatter.write_str("definition-local graph is invalid"),
            Self::InvalidInstancePath => formatter.write_str("instance path is unresolved"),
            Self::IdExhausted => formatter.write_str("canonical ID space is exhausted"),
            Self::WrongNodeKind(id) => write!(formatter, "node {} has the wrong kind", id.0),
            Self::OverrideNotFound(id) => write!(formatter, "override {id} does not exist"),
            Self::JointNotFound(id) => write!(formatter, "joint {} does not exist", id.0),
            Self::UndeclaredOverrideParameter => {
                formatter.write_str("override parameter is not declared by the root rule")
            }
            Self::UnresolvedDerivedOutput => {
                formatter.write_str("derived output is unresolved or ambiguous")
            }
            Self::EvaluationEnvelopeMismatch => {
                formatter.write_str("evaluation envelope does not match the current snapshot")
            }
            Self::EvaluationEvidenceMismatch => {
                formatter.write_str("evaluation evidence does not match current evaluation")
            }
            Self::FailedEvaluation(id) => write!(formatter, "node {} evaluation failed", id.0),
            Self::RevisionExhausted => formatter.write_str("revision ID space is exhausted"),
            Self::Graph(error) => error.fmt(formatter),
            Self::Prismatic(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CanonicalError {}

impl From<PrismaticError> for CanonicalError {
    fn from(error: PrismaticError) -> Self {
        Self::Prismatic(error)
    }
}

fn ensure_product_id(id: u64) -> Result<(), CanonicalError> {
    if id == 0 {
        Err(CanonicalError::ReservedProductId)
    } else {
        Ok(())
    }
}

fn ensure_name(name: &str) -> Result<(), CanonicalError> {
    if name.trim().is_empty() {
        Err(CanonicalError::EmptyProductName)
    } else {
        Ok(())
    }
}

fn validate_transform(transform: Transform) -> Result<(), CanonicalError> {
    Transform::from_matrix(transform.matrix).map(|_| ())
}

fn validate_feature_kind(kind: &FeatureKind) -> Result<(), CanonicalError> {
    match kind {
        FeatureKind::Profile { points_mm } => {
            if points_mm.len() < 3
                || points_mm
                    .iter()
                    .flatten()
                    .any(|coordinate| !coordinate.is_finite())
            {
                return Err(CanonicalError::InvalidProfile);
            }
            let twice_area: f64 = points_mm
                .iter()
                .zip(points_mm.iter().cycle().skip(1))
                .take(points_mm.len())
                .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
                .sum();
            if twice_area.abs() <= f64::EPSILON {
                return Err(CanonicalError::InvalidProfile);
            }
            Ok(())
        }
        FeatureKind::Extrusion { height, .. } => {
            Dimension::new(height.source_token.clone(), height.millimetres).map(|_| ())
        }
    }
}

fn clone_definition_and_repoint(
    product: &mut ProductModel,
    plan: &CloneDefinitionPlan,
) -> Result<(), CanonicalError> {
    let occurrence_id = plan.occurrence_id;
    let source_definition_id = plan.source_definition_id;
    let new_definition_id = plan.new_definition_id;
    let new_definition_name = &plan.new_definition_name;
    let feature_id_map = &plan.feature_id_map;
    ensure_product_id(new_definition_id.0)?;
    ensure_name(new_definition_name)?;
    if product.definitions.contains_key(&new_definition_id) {
        return Err(CanonicalError::DefinitionAlreadyExists(new_definition_id));
    }
    let occurrence = product
        .occurrences
        .get(&occurrence_id)
        .ok_or(CanonicalError::OccurrenceNotFound(occurrence_id))?
        .as_ref()
        .clone();
    if occurrence.definition_id != source_definition_id {
        return Err(CanonicalError::OccurrenceDefinitionMismatch);
    }
    let source = product
        .definitions
        .get(&source_definition_id)
        .ok_or(CanonicalError::DefinitionNotFound(source_definition_id))?
        .as_ref()
        .clone();
    if feature_id_map.len() != source.feature_ids.len()
        || feature_id_map
            .iter()
            .map(|(source_id, _)| *source_id)
            .ne(source.feature_ids.iter().copied())
    {
        return Err(CanonicalError::InvalidFeatureMap);
    }
    let mut mapped_ids = BTreeSet::new();
    let mut mapping = BTreeMap::new();
    for (source_id, new_id) in feature_id_map {
        ensure_product_id(new_id.0)?;
        if !mapped_ids.insert(*new_id) || product.features.contains_key(new_id) {
            return Err(CanonicalError::InvalidFeatureMap);
        }
        mapping.insert(*source_id, *new_id);
    }

    let mut cloned_features = Vec::with_capacity(feature_id_map.len());
    for (source_id, new_id) in feature_id_map {
        let source_feature = product
            .features
            .get(source_id)
            .ok_or(CanonicalError::FeatureNotFound(*source_id))?;
        let kind = match &source_feature.kind {
            FeatureKind::Profile { points_mm } => FeatureKind::Profile {
                points_mm: points_mm.clone(),
            },
            FeatureKind::Extrusion { profile, height } => FeatureKind::Extrusion {
                profile: *mapping
                    .get(profile)
                    .ok_or(CanonicalError::InvalidFeatureMap)?,
                height: height.clone(),
            },
        };
        cloned_features.push(Arc::new(Feature {
            id: *new_id,
            definition_id: new_definition_id,
            name: source_feature.name.clone(),
            kind,
        }));
    }

    product.definitions.insert(
        new_definition_id,
        Arc::new(Definition {
            id: new_definition_id,
            name: new_definition_name.to_owned(),
            feature_ids: feature_id_map.iter().map(|(_, new_id)| *new_id).collect(),
            local_occurrence_ids: source.local_occurrence_ids.clone(),
            local_group_ids: source.local_group_ids.clone(),
        }),
    );
    for local_id in &source.local_group_ids {
        let old_key = LocalGroupKey {
            definition_id: source_definition_id,
            local_id: *local_id,
        };
        let local = product.local_groups[&old_key].as_ref();
        let key = LocalGroupKey {
            definition_id: new_definition_id,
            local_id: *local_id,
        };
        product.local_groups.insert(
            key,
            Arc::new(LocalGroup {
                key,
                name: local.name.clone(),
                transform: local.transform,
                parent: local.parent,
            }),
        );
    }
    for local_id in &source.local_occurrence_ids {
        let old_key = LocalOccurrenceKey {
            definition_id: source_definition_id,
            local_id: *local_id,
        };
        let local = product.local_occurrences[&old_key].as_ref();
        let key = LocalOccurrenceKey {
            definition_id: new_definition_id,
            local_id: *local_id,
        };
        product.local_occurrences.insert(
            key,
            Arc::new(LocalOccurrence {
                key,
                definition_id: local.definition_id,
                name: local.name.clone(),
                transform: local.transform,
                parent: local.parent,
                tag: local.tag,
                visible: local.visible,
            }),
        );
    }
    for feature in cloned_features {
        product.features.insert(feature.id, feature);
    }
    product.occurrences.insert(
        occurrence_id,
        Arc::new(Occurrence {
            definition_id: new_definition_id,
            ..occurrence
        }),
    );
    Ok(())
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, CanonicalError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(CanonicalError::IdExhausted)
}

fn group_is_descendant(product: &ProductModel, root: GroupId, target: GroupId) -> bool {
    let mut cursor = Some(target);
    while let Some(candidate) = cursor {
        if candidate == root {
            return true;
        }
        cursor = product
            .groups
            .get(&candidate)
            .and_then(|group| group.parent);
    }
    false
}

fn descendant_groups(
    product: &ProductModel,
    root: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    if !product.groups.contains_key(&root) {
        return Err(CanonicalError::GroupNotFound(root));
    }
    Ok(product
        .groups
        .keys()
        .copied()
        .filter(|id| group_is_descendant(product, root, *id))
        .collect())
}

fn world_group_lineage(
    product: &ProductModel,
    target: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    let mut lineage = Vec::new();
    let mut cursor = Some(target);
    while let Some(id) = cursor {
        let group = product
            .groups
            .get(&id)
            .ok_or(CanonicalError::GroupNotFound(id))?;
        lineage.push(id);
        cursor = group.parent;
    }
    lineage.reverse();
    Ok(lineage)
}

fn group_lineage(
    product: &ProductModel,
    root: GroupId,
    target: GroupId,
) -> Result<Vec<GroupId>, CanonicalError> {
    let mut lineage = Vec::new();
    let mut cursor = Some(target);
    while let Some(id) = cursor {
        lineage.push(id);
        if id == root {
            lineage.reverse();
            return Ok(lineage);
        }
        cursor = product.groups.get(&id).and_then(|group| group.parent);
    }
    Err(CanonicalError::InvalidLocalGraph)
}

fn conversion_mappings(
    product: &ProductModel,
    plan: &ConvertGroupPlan,
) -> Result<Vec<ConversionMapping>, CanonicalError> {
    let converted_groups = descendant_groups(product, plan.group_id)?;
    let converted_group_set: BTreeSet<_> = converted_groups.iter().copied().collect();
    let mut mappings = Vec::new();
    for id in product.groups.keys().copied() {
        let old_path = WorldEntityPath {
            groups: world_group_lineage(product, id)?,
            occurrence: None,
        };
        let resolution = if converted_group_set.contains(&id) {
            let converted_lineage = group_lineage(product, plan.group_id, id)?;
            let mut new_path = InstancePath::root(plan.new_occurrence_id);
            new_path.steps.extend(
                converted_lineage
                    .iter()
                    .skip(1)
                    .map(|id| InstancePathStep::Group(LocalGroupId(id.0))),
            );
            MappingResolution::Resolved {
                new_id: if id == plan.group_id {
                    ConvertedEntityId::ComponentOccurrence(plan.new_occurrence_id)
                } else {
                    ConvertedEntityId::LocalGroup(LocalGroupKey {
                        definition_id: plan.new_definition_id,
                        local_id: LocalGroupId(id.0),
                    })
                },
                new_path,
            }
        } else {
            MappingResolution::Unresolved {
                reason: UnresolvedMappingReason::NotInConvertedGroup,
            }
        };
        mappings.push(ConversionMapping {
            old_id: WorldEntityId::Group(id),
            old_path,
            resolution,
        });
    }
    for occurrence in product.occurrences.values() {
        let old_groups = occurrence.parent.map_or_else(
            || Ok(Vec::new()),
            |parent| world_group_lineage(product, parent),
        )?;
        let old_path = WorldEntityPath {
            groups: old_groups,
            occurrence: Some(occurrence.id),
        };
        let resolution = if occurrence
            .parent
            .is_some_and(|parent| converted_group_set.contains(&parent))
        {
            let converted_lineage =
                group_lineage(product, plan.group_id, occurrence.parent.unwrap())?;
            let mut new_path = InstancePath::root(plan.new_occurrence_id);
            new_path.steps.extend(
                converted_lineage
                    .iter()
                    .skip(1)
                    .map(|id| InstancePathStep::Group(LocalGroupId(id.0))),
            );
            new_path
                .steps
                .push(InstancePathStep::Occurrence(LocalOccurrenceId(
                    occurrence.id.0,
                )));
            MappingResolution::Resolved {
                new_id: ConvertedEntityId::LocalOccurrence(LocalOccurrenceKey {
                    definition_id: plan.new_definition_id,
                    local_id: LocalOccurrenceId(occurrence.id.0),
                }),
                new_path,
            }
        } else {
            MappingResolution::Unresolved {
                reason: UnresolvedMappingReason::NotInConvertedGroup,
            }
        };
        mappings.push(ConversionMapping {
            old_id: WorldEntityId::Occurrence(occurrence.id),
            old_path,
            resolution,
        });
    }
    mappings.sort_by(|left, right| left.old_path.cmp(&right.old_path));
    Ok(mappings)
}

fn convert_group_to_component_model(
    product: &mut ProductModel,
    plan: &ConvertGroupPlan,
) -> Result<(), CanonicalError> {
    ensure_name(&plan.component_name)?;
    if product.definitions.contains_key(&plan.new_definition_id) {
        return Err(CanonicalError::DefinitionAlreadyExists(
            plan.new_definition_id,
        ));
    }
    if product.occurrences.contains_key(&plan.new_occurrence_id) {
        return Err(CanonicalError::OccurrenceAlreadyExists(
            plan.new_occurrence_id,
        ));
    }
    let root = product
        .groups
        .get(&plan.group_id)
        .ok_or(CanonicalError::GroupNotFound(plan.group_id))?
        .as_ref()
        .clone();
    let groups = descendant_groups(product, plan.group_id)?;
    let group_set: BTreeSet<_> = groups.iter().copied().collect();
    let occurrence_ids: Vec<_> = product
        .occurrences
        .values()
        .filter(|item| {
            item.parent
                .is_some_and(|parent| group_set.contains(&parent))
        })
        .map(|item| item.id)
        .collect();
    let local_group_ids = groups
        .iter()
        .copied()
        .filter(|id| *id != plan.group_id)
        .map(|id| LocalGroupId(id.0))
        .collect::<Vec<_>>();
    let local_occurrence_ids = occurrence_ids
        .iter()
        .map(|id| LocalOccurrenceId(id.0))
        .collect::<Vec<_>>();
    product.definitions.insert(
        plan.new_definition_id,
        Arc::new(Definition {
            id: plan.new_definition_id,
            name: plan.component_name.clone(),
            feature_ids: Vec::new(),
            local_occurrence_ids: local_occurrence_ids.clone(),
            local_group_ids: local_group_ids.clone(),
        }),
    );
    for id in groups.iter().copied().filter(|id| *id != plan.group_id) {
        let group = product.groups[&id].as_ref();
        let key = LocalGroupKey {
            definition_id: plan.new_definition_id,
            local_id: LocalGroupId(id.0),
        };
        product.local_groups.insert(
            key,
            Arc::new(LocalGroup {
                key,
                name: group.name.clone(),
                transform: group.transform,
                parent: group
                    .parent
                    .filter(|parent| *parent != plan.group_id)
                    .map(|parent| LocalGroupId(parent.0)),
            }),
        );
    }
    for id in &occurrence_ids {
        let occurrence = product.occurrences[id].as_ref();
        let key = LocalOccurrenceKey {
            definition_id: plan.new_definition_id,
            local_id: LocalOccurrenceId(id.0),
        };
        product.local_occurrences.insert(
            key,
            Arc::new(LocalOccurrence {
                key,
                definition_id: occurrence.definition_id,
                name: occurrence.name.clone(),
                transform: occurrence.transform,
                parent: occurrence
                    .parent
                    .filter(|parent| *parent != plan.group_id)
                    .map(|parent| LocalGroupId(parent.0)),
                tag: occurrence.tag,
                visible: occurrence.visible,
            }),
        );
    }
    for id in occurrence_ids {
        product.occurrences.remove(&id);
    }
    for id in groups {
        product.groups.remove(&id);
    }
    product.occurrences.insert(
        plan.new_occurrence_id,
        Arc::new(Occurrence {
            id: plan.new_occurrence_id,
            definition_id: plan.new_definition_id,
            name: plan.component_name.clone(),
            transform: root.transform,
            parent: root.parent,
            tag: None,
            visible: true,
        }),
    );
    Ok(())
}

fn refresh_override_health(product: &mut ProductModel) {
    for value in product.overrides.values_mut() {
        let audited = resolve_derived_identity(&product.evaluator_nodes, &value.target);
        if value.health != audited {
            let mut refreshed = value.as_ref().clone();
            refreshed.health = audited;
            *value = Arc::new(refreshed);
        }
    }
}

fn validate_overrides(product: &ProductModel) -> Result<(), CanonicalError> {
    for value in product.overrides.values() {
        if let Some(root) = product.evaluator_nodes.get(&value.target.root_rule_node_id) {
            if !root
                .allowed_parameters()
                .iter()
                .any(|parameter| parameter.name() == value.parameter)
            {
                return Err(CanonicalError::UndeclaredOverrideParameter);
            }
        }
    }
    Ok(())
}

fn validate_product(product: &ProductModel) -> Result<(), CanonicalError> {
    ensure_product_id(product.document_id.0)?;
    for (id, joint) in &product.joints {
        if *id != joint.id() || !joint.volume().has_positive_volume() {
            return Err(CanonicalError::Prismatic(PrismaticError::EmptyVolume));
        }
    }
    for definition in product.definitions.values() {
        ensure_product_id(definition.id.0)?;
        ensure_name(&definition.name)?;
        let mut seen = BTreeSet::new();
        for feature_id in &definition.feature_ids {
            if !seen.insert(*feature_id) {
                return Err(CanonicalError::InvalidFeatureOwnership(*feature_id));
            }
            let feature = product
                .features
                .get(feature_id)
                .ok_or(CanonicalError::FeatureNotFound(*feature_id))?;
            if feature.definition_id != definition.id {
                return Err(CanonicalError::InvalidFeatureOwnership(*feature_id));
            }
        }
        let mut local_ids = BTreeSet::new();
        for local_id in &definition.local_group_ids {
            if !local_ids.insert(local_id.0)
                || !product.local_groups.contains_key(&LocalGroupKey {
                    definition_id: definition.id,
                    local_id: *local_id,
                })
            {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        for local_id in &definition.local_occurrence_ids {
            if !local_ids.insert(local_id.0)
                || !product.local_occurrences.contains_key(&LocalOccurrenceKey {
                    definition_id: definition.id,
                    local_id: *local_id,
                })
            {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
    }
    for feature in product.features.values() {
        ensure_product_id(feature.id.0)?;
        ensure_name(&feature.name)?;
        validate_feature_kind(&feature.kind)?;
        let definition = product
            .definitions
            .get(&feature.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(feature.definition_id))?;
        if !definition.feature_ids.contains(&feature.id) {
            return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
        }
        if let FeatureKind::Extrusion { profile, .. } = feature.kind {
            let profile = product
                .features
                .get(&profile)
                .ok_or(CanonicalError::FeatureNotFound(profile))?;
            if profile.definition_id != feature.definition_id
                || !matches!(profile.kind, FeatureKind::Profile { .. })
            {
                return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
            }
        }
    }
    for occurrence in product.occurrences.values() {
        ensure_product_id(occurrence.id.0)?;
        ensure_name(&occurrence.name)?;
        validate_transform(occurrence.transform)?;
        if !product.definitions.contains_key(&occurrence.definition_id) {
            return Err(CanonicalError::DefinitionNotFound(occurrence.definition_id));
        }
        if let Some(parent) = occurrence.parent
            && !product.groups.contains_key(&parent)
        {
            return Err(CanonicalError::GroupNotFound(parent));
        }
    }
    for group in product.groups.values() {
        ensure_product_id(group.id.0)?;
        ensure_name(&group.name)?;
        validate_transform(group.transform)?;
        if let Some(parent) = group.parent
            && !product.groups.contains_key(&parent)
        {
            return Err(CanonicalError::GroupNotFound(parent));
        }
        let mut visiting = BTreeSet::new();
        let mut cursor = Some(group.id);
        while let Some(group_id) = cursor {
            if !visiting.insert(group_id) {
                return Err(CanonicalError::GroupCycle(group_id));
            }
            cursor = product.groups[&group_id].parent;
        }
    }
    for (key, group) in &product.local_groups {
        if key != &group.key
            || !product.definitions.contains_key(&key.definition_id)
            || !product.definitions[&key.definition_id]
                .local_group_ids
                .contains(&key.local_id)
        {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        if let Some(parent) = group.parent {
            let parent_key = LocalGroupKey {
                definition_id: key.definition_id,
                local_id: parent,
            };
            if !product.local_groups.contains_key(&parent_key) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        let mut visiting = BTreeSet::new();
        let mut cursor = Some(key.local_id);
        while let Some(local_id) = cursor {
            if !visiting.insert(local_id) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
            cursor = product.local_groups[&LocalGroupKey {
                definition_id: key.definition_id,
                local_id,
            }]
                .parent;
        }
    }
    for (key, occurrence) in &product.local_occurrences {
        if key != &occurrence.key
            || !product.definitions.contains_key(&key.definition_id)
            || !product.definitions[&key.definition_id]
                .local_occurrence_ids
                .contains(&key.local_id)
            || !product.definitions.contains_key(&occurrence.definition_id)
        {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        if let Some(parent) = occurrence.parent {
            let parent_key = LocalGroupKey {
                definition_id: key.definition_id,
                local_id: parent,
            };
            if !product.local_groups.contains_key(&parent_key) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
    }
    validate_definition_ownership_graph(product)
}

fn validate_definition_ownership_graph(product: &ProductModel) -> Result<(), CanonicalError> {
    fn visit(
        definition_id: DefinitionId,
        product: &ProductModel,
        visiting: &mut BTreeSet<DefinitionId>,
        visited: &mut BTreeSet<DefinitionId>,
    ) -> Result<(), CanonicalError> {
        if visited.contains(&definition_id) {
            return Ok(());
        }
        if !visiting.insert(definition_id) {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        let definition = &product.definitions[&definition_id];
        for local_id in &definition.local_occurrence_ids {
            let target = product.local_occurrences[&LocalOccurrenceKey {
                definition_id,
                local_id: *local_id,
            }]
                .definition_id;
            visit(target, product, visiting, visited)?;
        }
        visiting.remove(&definition_id);
        visited.insert(definition_id);
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for definition_id in product.definitions.keys().copied() {
        visit(definition_id, product, &mut BTreeSet::new(), &mut visited)?;
    }
    Ok(())
}

fn authoritative_dependencies(
    snapshot: &Snapshot,
    batch: &CommandBatch,
) -> BTreeSet<AuthoritativeDependency> {
    let mut dependencies = BTreeSet::new();
    for command in &batch.commands {
        match command {
            CanonicalCommand::CreateEvaluatorNode {
                id,
                dependencies: node_dependencies,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                for dependency in node_dependencies {
                    add_evaluator_dependency_closure(snapshot, *dependency, &mut dependencies);
                }
            }
            CanonicalCommand::SetEvaluatorDimension { id, .. }
            | CanonicalCommand::RenameEvaluatorNode { id, .. } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::CreateDefinition { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::DeleteDefinition { id } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
                dependencies.insert(AuthoritativeDependency::DefinitionUsers(*id));
            }
            CanonicalCommand::RenameDefinition { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::CreateFeature {
                id,
                definition_id,
                kind,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Feature(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                if let FeatureKind::Extrusion { profile, .. } = kind {
                    add_feature_dependency_closure(snapshot, *profile, &mut dependencies);
                }
            }
            CanonicalCommand::DeleteFeature { id } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
                dependencies.insert(AuthoritativeDependency::FeatureUsers(*id));
            }
            CanonicalCommand::SetFeatureDimension { id, .. }
            | CanonicalCommand::SetProfilePoints { id, .. } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                parent,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::DeleteOccurrence { id }
            | CanonicalCommand::SetOccurrenceTransform { id, .. }
            | CanonicalCommand::SetOccurrenceVisibility { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
            }
            CanonicalCommand::RepointOccurrence { id, definition_id } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::SetOccurrenceParent { id, parent } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::CreateGroup { id, parent, .. } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::DeleteGroup { id } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                dependencies.insert(AuthoritativeDependency::GroupChildren(*id));
            }
            CanonicalCommand::SetGroupTransform { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
            }
            CanonicalCommand::SetGroupParent { id, parent } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.occurrence_id));
                dependencies.insert(AuthoritativeDependency::Definition(
                    plan.source_definition_id,
                ));
                dependencies.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                for (source_id, new_id) in &plan.feature_id_map {
                    add_feature_dependency_closure(snapshot, *source_id, &mut dependencies);
                    dependencies.insert(AuthoritativeDependency::Feature(*new_id));
                }
                if let Some(definition) = snapshot.definition(plan.source_definition_id) {
                    for local_id in definition.local_group_ids() {
                        dependencies.insert(AuthoritativeDependency::LocalGroup(LocalGroupKey {
                            definition_id: plan.source_definition_id,
                            local_id: *local_id,
                        }));
                    }
                    for local_id in definition.local_occurrence_ids() {
                        dependencies.insert(AuthoritativeDependency::LocalOccurrence(
                            LocalOccurrenceKey {
                                definition_id: plan.source_definition_id,
                                local_id: *local_id,
                            },
                        ));
                    }
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                dependencies.insert(AuthoritativeDependency::GroupSubtree(plan.group_id));
                dependencies.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.new_occurrence_id));
            }
            CanonicalCommand::CreateExpressionNode { id, .. } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
            }
            CanonicalCommand::CreateRuleNode { id, .. } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
            }
            CanonicalCommand::SetNodeExpression { id, .. }
            | CanonicalCommand::SetRuleOutputs { id, .. } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::UpsertOverride(value) => {
                dependencies.insert(AuthoritativeDependency::Override(value.id));
                add_evaluator_dependency_closure(
                    snapshot,
                    value.target.root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteOverride { id } => {
                dependencies.insert(AuthoritativeDependency::Override(*id));
            }
            CanonicalCommand::UpsertJoint(joint) => {
                dependencies.insert(AuthoritativeDependency::Joint(joint.id()));
            }
            CanonicalCommand::DeleteJoint { id } => {
                dependencies.insert(AuthoritativeDependency::Joint(*id));
            }
        }
    }
    dependencies
}

fn add_evaluator_dependency_closure(
    snapshot: &Snapshot,
    id: NodeId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    if !dependencies.insert(AuthoritativeDependency::EvaluatorNode(id)) {
        return;
    }
    if let Some(node) = snapshot.evaluator_node(id) {
        for dependency in node.dependencies() {
            add_evaluator_dependency_closure(snapshot, *dependency, dependencies);
        }
    }
}

fn add_feature_dependency_closure(
    snapshot: &Snapshot,
    id: FeatureId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    if !dependencies.insert(AuthoritativeDependency::Feature(id)) {
        return;
    }
    if let Some(feature) = snapshot.feature(id) {
        dependencies.insert(AuthoritativeDependency::Definition(feature.definition_id()));
        if let FeatureKind::Extrusion { profile, .. } = feature.kind() {
            add_feature_dependency_closure(snapshot, *profile, dependencies);
        }
    }
}

fn add_group_ancestry(
    snapshot: &Snapshot,
    mut group_id: Option<GroupId>,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    while let Some(id) = group_id {
        if !dependencies.insert(AuthoritativeDependency::Group(id)) {
            break;
        }
        group_id = snapshot.group(id).and_then(Group::parent);
    }
}

fn dependency_digest(
    snapshot: &Snapshot,
    dependencies: &BTreeSet<AuthoritativeDependency>,
) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.authoritative-dependencies.v1");
    digest.u64(dependencies.len() as u64);
    for dependency in dependencies {
        digest.authoritative_dependency(snapshot.product(), *dependency);
    }
    digest.finish()
}

fn dependent_closure(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
    changed: &BTreeSet<NodeId>,
) -> BTreeSet<NodeId> {
    let mut closure = changed.clone();
    loop {
        let before = closure.len();
        for (id, node) in nodes {
            if node
                .dependencies
                .iter()
                .any(|dependency| closure.contains(dependency))
            {
                closure.insert(*id);
            }
        }
        if closure.len() == before {
            return closure;
        }
    }
}

pub(crate) fn validate_graph(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
) -> Result<(), CanonicalError> {
    validate_typed_graph(nodes).map_err(CanonicalError::Graph)
}

fn digest_snapshot(snapshot: &Snapshot) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.document.v3");
    digest.u64(snapshot.product.document_id.0);
    digest.byte(match snapshot.product.units {
        UnitSystem::Millimetres => 1,
    });
    digest.u64(snapshot.product.evaluator_nodes.len() as u64);
    for node in snapshot.product.evaluator_nodes.values() {
        digest.node(node);
    }
    digest.u64(snapshot.product.overrides.len() as u64);
    for value in snapshot.product.overrides.values() {
        digest.canonical_override(value);
    }
    digest.u64(snapshot.product.joints.len() as u64);
    for joint in snapshot.product.joints.values() {
        digest.joint(joint);
    }
    digest.u64(snapshot.product.definitions.len() as u64);
    for definition in snapshot.product.definitions.values() {
        digest.definition(definition);
    }
    digest.u64(snapshot.product.features.len() as u64);
    for feature in snapshot.product.features.values() {
        digest.feature(feature);
    }
    digest.u64(snapshot.product.occurrences.len() as u64);
    for occurrence in snapshot.product.occurrences.values() {
        digest.occurrence(occurrence);
    }
    digest.u64(snapshot.product.groups.len() as u64);
    for group in snapshot.product.groups.values() {
        digest.group(group);
    }
    digest.u64(snapshot.product.local_groups.len() as u64);
    for group in snapshot.product.local_groups.values() {
        digest.local_group(group);
    }
    digest.u64(snapshot.product.local_occurrences.len() as u64);
    for occurrence in snapshot.product.local_occurrences.values() {
        digest.local_occurrence(occurrence);
    }
    digest.finish()
}

struct StableDigest(u64);

impl StableDigest {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for byte in bytes {
            self.byte(*byte);
        }
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn node(&mut self, node: &EvaluatorNode) {
        self.bytes(&node.canonical_spec_bytes());
    }

    fn ports(&mut self, ports: &[PortSpec]) {
        self.u64(ports.len() as u64);
        for port in ports {
            self.bytes(port.name().as_bytes());
            self.byte(match port.value_type() {
                ValueType::Number => 1,
            });
        }
    }
    fn rule_outputs(&mut self, outputs: &[RuleOutput]) {
        self.u64(outputs.len() as u64);
        let mut stack = outputs.iter().rev().collect::<Vec<_>>();
        while let Some(output) = stack.pop() {
            let segment = output.segment();
            self.u64(segment.producer_rule_id.0);
            self.bytes(segment.output_port.as_bytes());
            self.bytes(segment.semantic_key.as_bytes());
            self.u64(output.children().len() as u64);
            stack.extend(output.children().iter().rev());
        }
    }

    fn slot_path(&mut self, path: &SlotPath) {
        self.u64(path.segments().len() as u64);
        for segment in path.segments() {
            self.u64(segment.producer_rule_id.0);
            self.bytes(segment.output_port.as_bytes());
            self.bytes(segment.semantic_key.as_bytes());
        }
    }

    fn canonical_override(&mut self, value: &CanonicalOverride) {
        self.u64(value.id);
        self.u64(value.target.root_rule_node_id.0);
        self.slot_path(&value.target.slot_path);
        self.bytes(value.parameter.as_bytes());
        self.u64(value.value_bits);
        match value.health {
            SlotResolution::Resolved => self.byte(1),
            SlotResolution::Ambiguous { segment_index } => {
                self.byte(2);
                self.u64(segment_index as u64);
            }
            SlotResolution::Lost { segment_index } => {
                self.byte(3);
                self.u64(segment_index as u64);
            }
        }
    }

    fn joint(&mut self, joint: &CanonicalJoint) {
        self.u64(joint.id().0);
        self.u64(joint.participant_a().root_rule_node_id.0);
        self.slot_path(&joint.participant_a().slot_path);
        self.u64(joint.participant_b().root_rule_node_id.0);
        self.slot_path(&joint.participant_b().slot_path);
        for value in joint.volume().min().into_iter().chain(joint.volume().max()) {
            self.u64(value.to_bits());
        }
    }

    fn transform(&mut self, transform: Transform) {
        for value in transform.matrix {
            self.u64(value.to_bits());
        }
    }

    fn definition(&mut self, definition: &Definition) {
        self.u64(definition.id.0);
        self.bytes(definition.name.as_bytes());
        self.u64(definition.feature_ids.len() as u64);
        for feature_id in &definition.feature_ids {
            self.u64(feature_id.0);
        }
        self.u64(definition.local_group_ids.len() as u64);
        for id in &definition.local_group_ids {
            self.u64(id.0);
        }
        self.u64(definition.local_occurrence_ids.len() as u64);
        for id in &definition.local_occurrence_ids {
            self.u64(id.0);
        }
    }

    fn feature_kind(&mut self, kind: &FeatureKind) {
        match kind {
            FeatureKind::Profile { points_mm } => {
                self.byte(1);
                self.u64(points_mm.len() as u64);
                for point in points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            FeatureKind::Extrusion { profile, height } => {
                self.byte(2);
                self.u64(profile.0);
                self.bytes(height.source_token.as_bytes());
                self.u64(height.millimetres.to_bits());
            }
        }
    }

    fn feature(&mut self, feature: &Feature) {
        self.u64(feature.id.0);
        self.u64(feature.definition_id.0);
        self.bytes(feature.name.as_bytes());
        self.feature_kind(&feature.kind);
    }

    fn occurrence(&mut self, occurrence: &Occurrence) {
        self.u64(occurrence.id.0);
        self.u64(occurrence.definition_id.0);
        self.bytes(occurrence.name.as_bytes());
        self.transform(occurrence.transform);
        self.optional_id(occurrence.parent.map(|id| id.0));
        self.optional_id(occurrence.tag.map(|id| id.0));
        self.byte(u8::from(occurrence.visible));
    }

    fn group(&mut self, group: &Group) {
        self.u64(group.id.0);
        self.bytes(group.name.as_bytes());
        self.transform(group.transform);
        self.optional_id(group.parent.map(|id| id.0));
    }

    fn local_group(&mut self, group: &LocalGroup) {
        self.u64(group.key.definition_id.0);
        self.u64(group.key.local_id.0);
        self.bytes(group.name.as_bytes());
        self.transform(group.transform);
        self.optional_id(group.parent.map(|id| id.0));
    }

    fn local_occurrence(&mut self, occurrence: &LocalOccurrence) {
        self.u64(occurrence.key.definition_id.0);
        self.u64(occurrence.key.local_id.0);
        self.u64(occurrence.definition_id.0);
        self.bytes(occurrence.name.as_bytes());
        self.transform(occurrence.transform);
        self.optional_id(occurrence.parent.map(|id| id.0));
        self.optional_id(occurrence.tag.map(|id| id.0));
        self.byte(u8::from(occurrence.visible));
    }

    fn authoritative_dependency(
        &mut self,
        product: &ProductModel,
        dependency: AuthoritativeDependency,
    ) {
        match dependency {
            AuthoritativeDependency::EvaluatorNode(id) => {
                self.byte(1);
                self.u64(id.0);
                if let Some(node) = product.evaluator_nodes.get(&id) {
                    self.byte(1);
                    self.node(node);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Override(id) => {
                self.byte(12);
                self.u64(id);
                if let Some(value) = product.overrides.get(&id) {
                    self.byte(1);
                    self.canonical_override(value);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Joint(id) => {
                self.byte(13);
                self.u64(id.0);
                if let Some(joint) = product.joints.get(&id) {
                    self.byte(1);
                    self.joint(joint);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Definition(id) => {
                self.byte(2);
                self.u64(id.0);
                if let Some(definition) = product.definitions.get(&id) {
                    self.byte(1);
                    self.definition(definition);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Feature(id) => {
                self.byte(3);
                self.u64(id.0);
                if let Some(feature) = product.features.get(&id) {
                    self.byte(1);
                    self.feature(feature);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Occurrence(id) => {
                self.byte(4);
                self.u64(id.0);
                if let Some(occurrence) = product.occurrences.get(&id) {
                    self.byte(1);
                    self.occurrence(occurrence);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Group(id) => {
                self.byte(5);
                self.u64(id.0);
                if let Some(group) = product.groups.get(&id) {
                    self.byte(1);
                    self.group(group);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::LocalGroup(key) => {
                self.byte(6);
                self.u64(key.definition_id.0);
                self.u64(key.local_id.0);
                if let Some(group) = product.local_groups.get(&key) {
                    self.byte(1);
                    self.local_group(group);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::LocalOccurrence(key) => {
                self.byte(7);
                self.u64(key.definition_id.0);
                self.u64(key.local_id.0);
                if let Some(occurrence) = product.local_occurrences.get(&key) {
                    self.byte(1);
                    self.local_occurrence(occurrence);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::DefinitionUsers(id) => {
                self.byte(8);
                self.u64(id.0);
                let world_users = product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == id)
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>();
                self.u64(world_users.len() as u64);
                for occurrence_id in world_users {
                    self.u64(occurrence_id.0);
                }
                let local_users = product
                    .local_occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == id)
                    .map(|occurrence| occurrence.key)
                    .collect::<Vec<_>>();
                self.u64(local_users.len() as u64);
                for key in local_users {
                    self.u64(key.definition_id.0);
                    self.u64(key.local_id.0);
                }
            }
            AuthoritativeDependency::FeatureUsers(id) => {
                self.byte(9);
                self.u64(id.0);
                let users = product
                    .features
                    .values()
                    .filter_map(|feature| match feature.kind {
                        FeatureKind::Extrusion { profile, .. } if profile == id => Some(feature.id),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                self.u64(users.len() as u64);
                for feature_id in users {
                    self.u64(feature_id.0);
                }
            }
            AuthoritativeDependency::GroupChildren(id) => {
                self.byte(10);
                self.u64(id.0);
                let group_children = product
                    .groups
                    .values()
                    .filter(|group| group.parent == Some(id))
                    .map(|group| group.id)
                    .collect::<Vec<_>>();
                self.u64(group_children.len() as u64);
                for group_id in group_children {
                    self.u64(group_id.0);
                }
                let occurrence_children = product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.parent == Some(id))
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>();
                self.u64(occurrence_children.len() as u64);
                for occurrence_id in occurrence_children {
                    self.u64(occurrence_id.0);
                }
            }
            AuthoritativeDependency::GroupSubtree(root) => {
                self.byte(11);
                self.u64(root.0);
                let descendants = product
                    .groups
                    .keys()
                    .copied()
                    .filter(|id| group_is_descendant(product, root, *id))
                    .collect::<BTreeSet<_>>();
                self.u64(descendants.len() as u64);
                for id in &descendants {
                    self.group(&product.groups[id]);
                }
                let occurrences = product
                    .occurrences
                    .values()
                    .filter(|occurrence| {
                        occurrence
                            .parent
                            .is_some_and(|parent| descendants.contains(&parent))
                    })
                    .collect::<Vec<_>>();
                self.u64(occurrences.len() as u64);
                for occurrence in occurrences {
                    self.occurrence(occurrence);
                }
            }
        }
    }

    fn optional_id(&mut self, id: Option<u64>) {
        match id {
            Some(id) => {
                self.byte(1);
                self.u64(id);
            }
            None => self.byte(0),
        }
    }

    fn command(&mut self, command: &CanonicalCommand) {
        match command {
            CanonicalCommand::CreateEvaluatorNode {
                id,
                name,
                dimension,
                dependencies,
            } => {
                self.byte(1);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
                self.u64(dependencies.len() as u64);
                for dependency in dependencies {
                    self.u64(dependency.0);
                }
            }
            CanonicalCommand::SetEvaluatorDimension { id, dimension } => {
                self.byte(2);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::RenameEvaluatorNode { id, name } => {
                self.byte(3);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::CreateDefinition { id, name } => {
                self.byte(10);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::DeleteDefinition { id } => {
                self.byte(11);
                self.u64(id.0);
            }
            CanonicalCommand::RenameDefinition { id, name } => {
                self.byte(12);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::CreateFeature {
                id,
                definition_id,
                name,
                kind,
            } => {
                self.byte(13);
                self.u64(id.0);
                self.u64(definition_id.0);
                self.bytes(name.as_bytes());
                self.feature_kind(kind);
            }
            CanonicalCommand::DeleteFeature { id } => {
                self.byte(14);
                self.u64(id.0);
            }
            CanonicalCommand::SetFeatureDimension { id, dimension } => {
                self.byte(15);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::SetProfilePoints { id, points_mm } => {
                self.byte(27);
                self.u64(id.0);
                self.u64(points_mm.len() as u64);
                for point in points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                name,
                transform,
                parent,
                tag,
                visible,
            } => {
                self.byte(16);
                self.u64(id.0);
                self.u64(definition_id.0);
                self.bytes(name.as_bytes());
                self.transform(*transform);
                self.optional_id(parent.map(|id| id.0));
                self.optional_id(tag.map(|id| id.0));
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteOccurrence { id } => {
                self.byte(17);
                self.u64(id.0);
            }
            CanonicalCommand::SetOccurrenceTransform { id, transform } => {
                self.byte(18);
                self.u64(id.0);
                self.transform(*transform);
            }
            CanonicalCommand::SetOccurrenceVisibility { id, visible } => {
                self.byte(19);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::RepointOccurrence { id, definition_id } => {
                self.byte(20);
                self.u64(id.0);
                self.u64(definition_id.0);
            }
            CanonicalCommand::SetOccurrenceParent { id, parent } => {
                self.byte(21);
                self.u64(id.0);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::CreateGroup {
                id,
                name,
                transform,
                parent,
            } => {
                self.byte(22);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.transform(*transform);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::DeleteGroup { id } => {
                self.byte(23);
                self.u64(id.0);
            }
            CanonicalCommand::SetGroupTransform { id, transform } => {
                self.byte(24);
                self.u64(id.0);
                self.transform(*transform);
            }
            CanonicalCommand::SetGroupParent { id, parent } => {
                self.byte(25);
                self.u64(id.0);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                self.byte(26);
                self.u64(plan.occurrence_id.0);
                self.u64(plan.source_definition_id.0);
                self.u64(plan.new_definition_id.0);
                self.bytes(plan.new_definition_name.as_bytes());
                self.u64(plan.feature_id_map.len() as u64);
                for (source_id, new_id) in &plan.feature_id_map {
                    self.u64(source_id.0);
                    self.u64(new_id.0);
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                self.byte(28);
                self.u64(plan.group_id.0);
                self.u64(plan.new_definition_id.0);
                self.u64(plan.new_occurrence_id.0);
                self.bytes(plan.component_name.as_bytes());
            }
            CanonicalCommand::CreateExpressionNode {
                id,
                name,
                expression,
            } => {
                self.byte(4);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(expression.as_bytes());
            }
            CanonicalCommand::CreateRuleNode {
                id,
                name,
                expression,
                input_ports,
                output_ports,
                outputs,
                override_parameters,
            } => {
                self.byte(5);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(expression.as_bytes());
                self.ports(input_ports);
                self.ports(output_ports);
                self.rule_outputs(outputs);
                self.u64(override_parameters.len() as u64);
                for parameter in override_parameters {
                    self.bytes(parameter.name().as_bytes());
                    self.byte(match parameter.merge_policy() {
                        OverrideMergePolicy::Replace => 1,
                    });
                }
            }
            CanonicalCommand::SetNodeExpression { id, expression } => {
                self.byte(6);
                self.u64(id.0);
                self.bytes(expression.as_bytes());
            }
            CanonicalCommand::SetRuleOutputs { id, outputs } => {
                self.byte(7);
                self.u64(id.0);
                self.rule_outputs(outputs);
            }
            CanonicalCommand::UpsertOverride(value) => {
                self.byte(8);
                self.canonical_override(value);
            }
            CanonicalCommand::DeleteOverride { id } => {
                self.byte(9);
                self.u64(*id);
            }
            CanonicalCommand::UpsertJoint(joint) => {
                self.byte(29);
                self.joint(joint);
            }
            CanonicalCommand::DeleteJoint { id } => {
                self.byte(30);
                self.u64(id.0);
            }
        }
    }

    fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}
