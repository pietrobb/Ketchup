use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

pub const COMMAND_SCHEMA_V1: &str = "ketchup.command.v1";
pub const TOLERANCE_PROFILE_V1: &str = "ketchup.tolerance.r0-v1";
const MAX_CANONICAL_ABS_MM: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

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

#[derive(Clone)]
pub(crate) struct ProductModel {
    pub(crate) document_id: DocumentId,
    pub(crate) units: UnitSystem,
    pub(crate) definitions: BTreeMap<DefinitionId, Arc<Definition>>,
    pub(crate) features: BTreeMap<FeatureId, Arc<Feature>>,
    pub(crate) occurrences: BTreeMap<OccurrenceId, Arc<Occurrence>>,
    pub(crate) groups: BTreeMap<GroupId, Arc<Group>>,
}

impl Default for ProductModel {
    fn default() -> Self {
        Self {
            document_id: DocumentId(1),
            units: UnitSystem::Millimetres,
            definitions: BTreeMap::new(),
            features: BTreeMap::new(),
            occurrences: BTreeMap::new(),
            groups: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneOccurrence {
    pub occurrence_id: OccurrenceId,
    pub definition_id: DefinitionId,
    pub occurrence_name: String,
    pub definition_name: String,
    pub transform: Transform,
    pub parent: Option<GroupId>,
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
pub struct CanonicalNode {
    id: NodeId,
    name: String,
    dimension: Dimension,
    dependencies: Vec<NodeId>,
}

impl CanonicalNode {
    pub(crate) fn new(
        id: NodeId,
        name: String,
        dimension: Dimension,
        dependencies: Vec<NodeId>,
    ) -> Result<Self, CanonicalError> {
        if id.0 == 0 {
            return Err(CanonicalError::ReservedNodeId);
        }
        if name.trim().is_empty() {
            return Err(CanonicalError::EmptyNodeName);
        }
        if !dependencies.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(CanonicalError::DependenciesNotCanonical);
        }
        if dependencies.contains(&id) {
            return Err(CanonicalError::DependencyCycle(id));
        }
        Ok(Self {
            id,
            name,
            dimension,
            dependencies,
        })
    }

    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn dimension(&self) -> &Dimension {
        &self.dimension
    }

    #[must_use]
    pub fn dependencies(&self) -> &[NodeId] {
        &self.dependencies
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalCommand {
    CreateNode {
        id: NodeId,
        name: String,
        dimension: Dimension,
        dependencies: Vec<NodeId>,
    },
    SetDimension {
        id: NodeId,
        dimension: Dimension,
    },
    RenameNode {
        id: NodeId,
        name: String,
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
    CloneDefinitionAndRepoint {
        occurrence_id: OccurrenceId,
        source_definition_id: DefinitionId,
        new_definition_id: DefinitionId,
        new_definition_name: String,
        feature_id_map: Vec<(FeatureId, FeatureId)>,
    },
}

impl CanonicalCommand {
    #[must_use]
    pub const fn target(&self) -> NodeId {
        match self {
            Self::CreateNode { id, .. }
            | Self::SetDimension { id, .. }
            | Self::RenameNode { id, .. } => *id,
            _ => NodeId(0),
        }
    }

    #[must_use]
    const fn is_product_command(&self) -> bool {
        !matches!(
            self,
            Self::CreateNode { .. } | Self::SetDimension { .. } | Self::RenameNode { .. }
        )
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

#[derive(Clone)]
pub struct Snapshot {
    revision_id: u64,
    nodes: Arc<BTreeMap<NodeId, Arc<CanonicalNode>>>,
    product: Arc<ProductModel>,
}

impl Snapshot {
    #[must_use]
    pub const fn revision_id(&self) -> u64 {
        self.revision_id
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&CanonicalNode> {
        self.nodes.get(&id).map(Arc::as_ref)
    }

    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
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
    pub fn scene_query(&self) -> Vec<SceneOccurrence> {
        self.product
            .occurrences
            .values()
            .map(|occurrence| {
                let definition = &self.product.definitions[&occurrence.definition_id];
                let shared_occurrence_count = self
                    .product
                    .occurrences
                    .values()
                    .filter(|candidate| candidate.definition_id == definition.id)
                    .count();
                SceneOccurrence {
                    occurrence_id: occurrence.id,
                    definition_id: definition.id,
                    occurrence_name: occurrence.name.clone(),
                    definition_name: definition.name.clone(),
                    transform: self
                        .world_transform_for_occurrence(occurrence.id)
                        .expect("validated occurrence hierarchy has a world transform"),
                    parent: occurrence.parent,
                    visible: occurrence.visible,
                    shared_occurrence_count,
                }
            })
            .collect()
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

    #[must_use]
    pub fn shares_node_with(&self, other: &Self, id: NodeId) -> bool {
        match (self.nodes.get(&id), other.nodes.get(&id)) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub(crate) fn nodes(&self) -> &BTreeMap<NodeId, Arc<CanonicalNode>> {
        &self.nodes
    }

    pub(crate) fn product(&self) -> &ProductModel {
        &self.product
    }
}

#[derive(Clone)]
pub struct Revision {
    id: u64,
    snapshot: Snapshot,
    batch_digest: String,
    recomputed_nodes: BTreeSet<NodeId>,
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
}

pub struct DocumentStore {
    revisions: Vec<Arc<Revision>>,
    cursor: usize,
    next_revision_id: u64,
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::from_nodes(0, BTreeMap::new()).expect("an empty canonical document is valid")
    }

    pub(crate) fn from_nodes(
        revision_id: u64,
        nodes: BTreeMap<NodeId, Arc<CanonicalNode>>,
    ) -> Result<Self, CanonicalError> {
        Self::from_parts(revision_id, nodes, ProductModel::default())
    }

    pub(crate) fn from_parts(
        revision_id: u64,
        nodes: BTreeMap<NodeId, Arc<CanonicalNode>>,
        product: ProductModel,
    ) -> Result<Self, CanonicalError> {
        validate_graph(&nodes)?;
        validate_product(&product)?;
        let nodes = Arc::new(nodes);
        let snapshot = Snapshot {
            revision_id,
            nodes,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: String::new(),
            recomputed_nodes: BTreeSet::new(),
        });
        Ok(Self {
            revisions: vec![revision],
            cursor: 0,
            next_revision_id: revision_id + 1,
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
        let mut nodes = current.nodes.as_ref().clone();
        let mut product = current.product.as_ref().clone();
        let mut changed = BTreeSet::new();

        for command in &batch.commands {
            match command {
                CanonicalCommand::CreateNode {
                    id,
                    name,
                    dimension,
                    dependencies,
                } => {
                    if nodes.contains_key(id) {
                        return Err(CanonicalError::NodeAlreadyExists(*id));
                    }
                    for dependency in dependencies {
                        if !nodes.contains_key(dependency) {
                            return Err(CanonicalError::MissingDependency(*dependency));
                        }
                    }
                    let node = CanonicalNode::new(
                        *id,
                        name.clone(),
                        dimension.clone(),
                        dependencies.clone(),
                    )?;
                    nodes.insert(*id, Arc::new(node));
                    changed.insert(*id);
                }
                CanonicalCommand::SetDimension { id, dimension } => {
                    let existing = nodes.get(id).ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = CanonicalNode::new(
                        *id,
                        existing.name.clone(),
                        dimension.clone(),
                        existing.dependencies.clone(),
                    )?;
                    nodes.insert(*id, Arc::new(replacement));
                    changed.insert(*id);
                }
                CanonicalCommand::RenameNode { id, name } => {
                    let existing = nodes.get(id).ok_or(CanonicalError::NodeNotFound(*id))?;
                    let replacement = CanonicalNode::new(
                        *id,
                        name.clone(),
                        existing.dimension.clone(),
                        existing.dependencies.clone(),
                    )?;
                    nodes.insert(*id, Arc::new(replacement));
                    changed.insert(*id);
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
                CanonicalCommand::CloneDefinitionAndRepoint {
                    occurrence_id,
                    source_definition_id,
                    new_definition_id,
                    new_definition_name,
                    feature_id_map,
                } => {
                    clone_definition_and_repoint(
                        &mut product,
                        *occurrence_id,
                        *source_definition_id,
                        *new_definition_id,
                        new_definition_name,
                        feature_id_map,
                    )?;
                }
            }
        }

        validate_graph(&nodes)?;
        validate_product(&product)?;
        let recomputed_nodes = dependent_closure(&nodes, &changed);
        let revision_id = self.next_revision_id;
        let snapshot = Snapshot {
            revision_id,
            nodes: Arc::new(nodes),
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: batch.digest(),
            recomputed_nodes,
        });

        self.revisions.truncate(self.cursor + 1);
        self.revisions.push(Arc::clone(&revision));
        self.cursor += 1;
        self.next_revision_id += 1;
        Ok(revision)
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
        let read_set = authoritative_read_set(&snapshot, &batch);
        Proposal {
            provenance_revision: snapshot.revision_id,
            command_digest: batch.digest(),
            dependency_digest: dependency_digest(&snapshot, &read_set, &batch),
            read_set,
            batch,
        }
    }

    #[must_use]
    pub fn validate_proposal(&self, proposal: &Proposal) -> ProposalValidity {
        let snapshot = self.current();
        let command_matches = proposal.command_digest == proposal.batch.digest();
        let dependencies_match = proposal.dependency_digest
            == dependency_digest(&snapshot, &proposal.read_set, &proposal.batch);
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
    read_set: BTreeSet<NodeId>,
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
    pub const fn read_set(&self) -> &BTreeSet<NodeId> {
        &self.read_set
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
        }
    }
}

impl std::error::Error for CanonicalError {}

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
    occurrence_id: OccurrenceId,
    source_definition_id: DefinitionId,
    new_definition_id: DefinitionId,
    new_definition_name: &str,
    feature_id_map: &[(FeatureId, FeatureId)],
) -> Result<(), CanonicalError> {
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
        }),
    );
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

fn validate_product(product: &ProductModel) -> Result<(), CanonicalError> {
    ensure_product_id(product.document_id.0)?;
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
    Ok(())
}

fn authoritative_read_set(snapshot: &Snapshot, batch: &CommandBatch) -> BTreeSet<NodeId> {
    let mut read_set = BTreeSet::new();
    for command in &batch.commands {
        read_set.insert(command.target());
        if let CanonicalCommand::CreateNode { dependencies, .. } = command {
            read_set.extend(dependencies.iter().copied());
        }
    }

    let mut pending: Vec<NodeId> = read_set.iter().copied().collect();
    while let Some(id) = pending.pop() {
        if let Some(node) = snapshot.node(id) {
            for dependency in &node.dependencies {
                if read_set.insert(*dependency) {
                    pending.push(*dependency);
                }
            }
        }
    }
    read_set
}

fn dependency_digest(
    snapshot: &Snapshot,
    read_set: &BTreeSet<NodeId>,
    batch: &CommandBatch,
) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(COMMAND_SCHEMA_V1.as_bytes());
    digest.bytes(TOLERANCE_PROFILE_V1.as_bytes());
    if batch
        .commands
        .iter()
        .any(CanonicalCommand::is_product_command)
    {
        digest.bytes(snapshot.canonical_digest().as_bytes());
    }
    for id in read_set {
        digest.u64(id.0);
        if let Some(node) = snapshot.node(*id) {
            digest.byte(1);
            digest.node(node);
        } else {
            digest.byte(0);
        }
    }
    digest.finish()
}

fn dependent_closure(
    nodes: &BTreeMap<NodeId, Arc<CanonicalNode>>,
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
    nodes: &BTreeMap<NodeId, Arc<CanonicalNode>>,
) -> Result<(), CanonicalError> {
    for node in nodes.values() {
        for dependency in &node.dependencies {
            if !nodes.contains_key(dependency) {
                return Err(CanonicalError::MissingDependency(*dependency));
            }
        }
    }

    fn visit(
        id: NodeId,
        nodes: &BTreeMap<NodeId, Arc<CanonicalNode>>,
        visiting: &mut BTreeSet<NodeId>,
        visited: &mut BTreeSet<NodeId>,
    ) -> Result<(), CanonicalError> {
        if visited.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(CanonicalError::DependencyCycle(id));
        }
        for dependency in &nodes[&id].dependencies {
            visit(*dependency, nodes, visiting, visited)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for id in nodes.keys() {
        visit(*id, nodes, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn digest_snapshot(snapshot: &Snapshot) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.document.v2");
    digest.u64(snapshot.product.document_id.0);
    digest.byte(match snapshot.product.units {
        UnitSystem::Millimetres => 1,
    });
    digest.u64(snapshot.nodes.len() as u64);
    for node in snapshot.nodes.values() {
        digest.node(node);
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

    fn node(&mut self, node: &CanonicalNode) {
        self.u64(node.id.0);
        self.bytes(node.name.as_bytes());
        self.bytes(node.dimension.source_token.as_bytes());
        self.u64(node.dimension.millimetres.to_bits());
        self.u64(node.dependencies.len() as u64);
        for dependency in &node.dependencies {
            self.u64(dependency.0);
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
            CanonicalCommand::CreateNode {
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
            CanonicalCommand::SetDimension { id, dimension } => {
                self.byte(2);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::RenameNode { id, name } => {
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
            CanonicalCommand::CloneDefinitionAndRepoint {
                occurrence_id,
                source_definition_id,
                new_definition_id,
                new_definition_name,
                feature_id_map,
            } => {
                self.byte(26);
                self.u64(occurrence_id.0);
                self.u64(source_definition_id.0);
                self.u64(new_definition_id.0);
                self.bytes(new_definition_name.as_bytes());
                self.u64(feature_id_map.len() as u64);
                for (source_id, new_id) in feature_id_map {
                    self.u64(source_id.0);
                    self.u64(new_id.0);
                }
            }
        }
    }

    fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}
