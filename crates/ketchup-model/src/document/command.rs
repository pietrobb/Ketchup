use super::*;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum CanonicalCommand {
    /// Replace the model tolerance every check of the document uses.
    SetTolerance {
        tolerance: TolerancePolicy,
    },
    /// Set the world-Z support plane without changing any geometry.
    SetFloorHeight {
        z_mm: Option<f64>,
    },
    /// Replace explicit support anchors without grounding sibling instances.
    SetGroundedInstances {
        paths: BTreeSet<InstancePath>,
    },
    /// Replace declarations of physical contact; does not constrain motion.
    SetContactJoints {
        joints: Vec<ContactJoint>,
    },
    /// Assign a document-unique, machine-neutral code to one physical instance.
    SetProductionCode {
        instance_path: InstancePath,
        code: Option<String>,
    },
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
    DeleteEvaluatorNode {
        id: NodeId,
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
    UpsertFeatureParameterBinding(FeatureParameterBinding),
    DeleteFeatureParameterBinding {
        target: FeatureParameterTarget,
    },
    RecomputeFeatureParameters {
        identity: EvaluationIdentity,
        scope: FeatureParameterRecomputeScope,
    },
    UpsertJoint(CanonicalJoint),
    DeleteJoint {
        id: JointId,
    },
    UpsertSpace(CanonicalSpace),
    DeleteSpace {
        id: SpaceId,
    },
    UpsertClearanceVolume(CanonicalClearanceVolume),
    DeleteClearanceVolume {
        id: ClearanceVolumeId,
    },
    UpsertCamPlan(CamPlan),
    DeleteCamPlan {
        id: CamPlanId,
    },
    UpsertPinJoint(PinJointContract),
    DeletePinJoint {
        id: PinJointId,
    },
    SetAssemblyRecipe(AssemblyRecipe),
    ClearAssemblyRecipe,
    UpsertPersistentDimension(PersistentDimension),
    DeletePersistentDimension {
        id: PersistentDimensionId,
    },
    CreateTag {
        id: TagId,
        name: String,
        visible: bool,
    },
    DeleteTag {
        id: TagId,
    },
    SetTagVisibility {
        id: TagId,
        visible: bool,
    },
    SetTagName {
        id: TagId,
        name: String,
    },
    /// Create or replace a saved view; it changes nothing that is shown until activated.
    UpsertSavedView(SavedView),
    DeleteSavedView {
        id: SavedViewId,
    },
    UpsertClassificationDimension {
        id: ClassificationDimensionId,
        name: String,
        categories: Vec<(ClassificationCategoryId, String)>,
    },
    SetOccurrenceClassification {
        occurrence_id: OccurrenceId,
        dimension_id: ClassificationDimensionId,
        category_id: Option<ClassificationCategoryId>,
    },
    CreateCollection {
        id: CollectionId,
        name: String,
    },
    DeleteCollection {
        id: CollectionId,
    },
    SetCollectionOccurrences {
        id: CollectionId,
        occurrence_ids: Vec<OccurrenceId>,
    },
    RecordImport(ImportReceipt),
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
    CreateBody {
        definition_id: DefinitionId,
        id: BodyId,
        name: String,
        visible: bool,
    },
    DeleteBody {
        definition_id: DefinitionId,
        id: BodyId,
    },
    RenameBody {
        definition_id: DefinitionId,
        id: BodyId,
        name: String,
    },
    SetActiveBody {
        definition_id: DefinitionId,
        id: BodyId,
    },
    SetBodyVisibility {
        definition_id: DefinitionId,
        id: BodyId,
        visible: bool,
    },
    ConsumeBody {
        definition_id: DefinitionId,
        id: BodyId,
        by_feature_id: FeatureId,
    },
    SetFeatureBodyOwnership {
        id: FeatureId,
        ownership: FeatureBodyOwnership,
    },
    SetBodyFeatureSuppression {
        definition_id: DefinitionId,
        body_id: BodyId,
        suppressed_feature_ids: Vec<FeatureId>,
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
    SetFeatureParameter {
        target: FeatureParameterTarget,
        dimension: Dimension,
    },
    CreateSketchConstraint {
        id: FeatureId,
        constraint: SketchConstraint,
    },
    ReplaceSketchConstraint {
        id: FeatureId,
        constraint: SketchConstraint,
    },
    DeleteSketchConstraint {
        id: FeatureId,
        constraint_id: SketchConstraintId,
    },
    SetSketchConstraintDimension {
        id: FeatureId,
        constraint_id: SketchConstraintId,
        dimension: Dimension,
    },
    SplitSketchEntity {
        id: FeatureId,
        entity_id: ketchup_geometry::sketch::SketchEntityId,
        new_entity_id: ketchup_geometry::sketch::SketchEntityId,
        parameter: f64,
        joint_constraint_ids: Vec<SketchConstraintId>,
    },
    JoinSketchEntities {
        id: FeatureId,
        source_entity_id: ketchup_geometry::sketch::SketchEntityId,
        source_endpoint: SketchPointKind,
        consumed_entity_id: ketchup_geometry::sketch::SketchEntityId,
        consumed_endpoint: SketchPointKind,
    },
    TrimSketchEntity {
        id: FeatureId,
        entity_id: ketchup_geometry::sketch::SketchEntityId,
        start_parameter: f64,
        end_parameter: f64,
    },
    ExtendSketchEntity {
        id: FeatureId,
        entity_id: ketchup_geometry::sketch::SketchEntityId,
        endpoint: SketchPointKind,
        parameter: f64,
    },
    OffsetSketchEntity {
        id: FeatureId,
        entity_id: ketchup_geometry::sketch::SketchEntityId,
        new_entity_id: ketchup_geometry::sketch::SketchEntityId,
        distance_mm: f64,
        side: SketchOffsetSide,
    },
    ProjectSketchEntity {
        id: FeatureId,
        source_feature_id: FeatureId,
        source_entity_id: ketchup_geometry::sketch::SketchEntityId,
        new_entity_id: ketchup_geometry::sketch::SketchEntityId,
        projection_constraint_id: SketchConstraintId,
    },
    SetSketchEntityConstruction {
        id: FeatureId,
        entity_id: ketchup_geometry::sketch::SketchEntityId,
        construction: bool,
        construction_constraint_id: SketchConstraintId,
    },
    TranslateProfile {
        id: FeatureId,
        delta_mm: [f64; 2],
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
        tags: BTreeSet<TagId>,
        visible: bool,
    },
    DeleteOccurrence {
        id: OccurrenceId,
    },
    SetOccurrenceTransform {
        id: OccurrenceId,
        transform: Transform,
    },
    CreateLocalOccurrence {
        key: LocalOccurrenceKey,
        definition_id: DefinitionId,
        name: String,
        transform: Transform,
        parent: Option<LocalGroupId>,
        tags: BTreeSet<TagId>,
        visible: bool,
    },
    DeleteLocalOccurrence {
        key: LocalOccurrenceKey,
    },
    RepointLocalOccurrence {
        key: LocalOccurrenceKey,
        definition_id: DefinitionId,
    },
    SetLocalOccurrenceParent {
        key: LocalOccurrenceKey,
        parent: Option<LocalGroupId>,
    },
    SetLocalOccurrenceTransform {
        key: LocalOccurrenceKey,
        transform: Transform,
    },
    SetLocalOccurrenceColor {
        key: LocalOccurrenceKey,
        color: Option<[u8; 3]>,
    },
    RenameLocalOccurrence {
        key: LocalOccurrenceKey,
        name: String,
    },
    RenameEntity {
        id: OccurrenceId,
        name: String,
    },
    ApplyAssemblySolve {
        source_revision: u64,
        source_digest: String,
        transforms: Vec<(OccurrenceId, Transform)>,
        instance_transforms: Vec<(InstancePath, Transform)>,
    },
    GuardAssemblyRecompute {
        source_revision: u64,
        source_digest: String,
    },
    SetOccurrenceGrounded {
        id: OccurrenceId,
        grounded: bool,
    },
    CreateAssemblyMate(AssemblyMate),
    RebindAssemblyMate(AssemblyMate),
    SetAssemblyMateKind {
        id: AssemblyMateId,
        kind: AssemblyMateKind,
    },
    DeleteAssemblyMate {
        id: AssemblyMateId,
    },
    CreateAssemblyJoint(AssemblyJoint),
    SetAssemblyJointKind {
        id: AssemblyJointId,
        kind: AssemblyJointKind,
    },
    SetAssemblyJointPosition {
        id: AssemblyJointId,
        position: f64,
    },
    SetAssemblyJointLimits {
        id: AssemblyJointId,
        limits: Option<AssemblyJointLimits>,
    },
    DeleteAssemblyJoint {
        id: AssemblyJointId,
    },
    CreateAssemblyMotionCoupling(AssemblyMotionCoupling),
    UpdateAssemblyMotionCoupling(AssemblyMotionCoupling),
    DeleteAssemblyMotionCoupling {
        id: AssemblyMotionCouplingId,
    },
    CreateAssemblyMotionStudy(AssemblyMotionStudy),
    UpdateAssemblyMotionStudy(AssemblyMotionStudy),
    DeleteAssemblyMotionStudy {
        id: AssemblyMotionStudyId,
    },
    CreateMechanicalInterface(MechanicalInterface),
    UpdateMechanicalInterface(MechanicalInterface),
    DeleteMechanicalInterface {
        id: MechanicalInterfaceId,
    },
    CreateMechanicalCondition(MechanicalCondition),
    UpdateMechanicalCondition(MechanicalCondition),
    DeleteMechanicalCondition {
        id: MechanicalConditionId,
    },
    CreateDrawingSheet(DrawingSheet),
    UpdateDrawingSheet(DrawingSheet),
    DeleteDrawingSheet {
        id: DrawingSheetId,
    },
    SetOccurrenceColor {
        id: OccurrenceId,
        color: Option<[u8; 3]>,
    },
    SetOccurrenceVisibility {
        id: OccurrenceId,
        visible: bool,
    },
    SetOccurrenceTags {
        id: OccurrenceId,
        tags: BTreeSet<TagId>,
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
    CreateLocalGroup {
        key: LocalGroupKey,
        name: String,
        transform: Transform,
        parent: Option<LocalGroupId>,
    },
    DeleteLocalGroup {
        key: LocalGroupKey,
    },
    SetLocalGroupTransform {
        key: LocalGroupKey,
        transform: Transform,
    },
    SetLocalGroupParent {
        key: LocalGroupKey,
        parent: Option<LocalGroupId>,
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
    ApplySolidTool(SolidToolPlan),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct CloneDefinitionPlan {
    pub(super) occurrence_id: OccurrenceId,
    pub(super) source_definition_id: DefinitionId,
    pub(super) new_definition_id: DefinitionId,
    pub(super) new_definition_name: String,
    pub(super) feature_id_map: Vec<(FeatureId, FeatureId)>,
}

impl CloneDefinitionPlan {
    pub fn new(
        occurrence_id: OccurrenceId,
        source_definition_id: DefinitionId,
        new_definition_id: DefinitionId,
        new_definition_name: String,
        feature_id_map: Vec<(FeatureId, FeatureId)>,
    ) -> Self {
        Self {
            occurrence_id,
            source_definition_id,
            new_definition_id,
            new_definition_name,
            feature_id_map,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ConvertGroupPlan {
    pub(super) group_id: GroupId,
    pub(super) new_definition_id: DefinitionId,
    pub(super) new_occurrence_id: OccurrenceId,
    pub(super) component_name: String,
}

impl ConvertGroupPlan {
    pub fn new(
        group_id: GroupId,
        new_definition_id: DefinitionId,
        new_occurrence_id: OccurrenceId,
        component_name: String,
    ) -> Self {
        Self {
            group_id,
            new_definition_id,
            new_occurrence_id,
            component_name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SolidToolPlan {
    pub operation: BooleanOperation,
    pub target_occurrence_id: OccurrenceId,
    pub target_feature_id: FeatureId,
    pub tool_occurrence_id: OccurrenceId,
    pub tool_feature_id: FeatureId,
    pub result_definition_id: DefinitionId,
    pub result_feature_ids: Vec<FeatureId>,
    pub result_definition_name: String,
    pub result_feature_name: String,
    pub keep_tool: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewBodyFeaturePlan {
    pub definition_id: DefinitionId,
    pub body_id: BodyId,
    pub body_name: String,
    pub feature_id: FeatureId,
    pub feature_name: String,
    pub feature_kind: FeatureKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolBodyPolicy {
    Preserve,
    Consume,
}

impl CanonicalCommand {
    /// Whether the command changes only how the model is viewed: tag visibility and saved
    /// views. A batch of such commands keeps the document's program.
    #[must_use]
    pub const fn is_view_only(&self) -> bool {
        matches!(
            self,
            Self::SetTagVisibility { .. } | Self::UpsertSavedView(_) | Self::DeleteSavedView { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiBodyBooleanPlan {
    pub definition_id: DefinitionId,
    pub operation: BooleanOperation,
    pub target_body_id: BodyId,
    pub target_feature_id: FeatureId,
    pub tool_body_id: BodyId,
    pub tool_feature_id: FeatureId,
    pub result_feature_id: FeatureId,
    pub result_feature_name: String,
    pub tool_policy: ToolBodyPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum AuthoritativeDependency {
    EvaluatorNode(NodeId),
    Override(u64),
    FeatureParameterBinding(FeatureParameterTarget),
    Joint(JointId),
    Space(SpaceId),
    ClearanceVolume(ClearanceVolumeId),
    CamPlan(CamPlanId),
    PinJoint(PinJointId),
    Tolerance,
    FloorHeight,
    GroundedInstances,
    ContactJoints,
    ProductionCodes,
    AssemblyRecipe,
    PersistentDimension(PersistentDimensionId),
    Tag(TagId),
    SavedView(SavedViewId),
    ClassificationDimension(ClassificationDimensionId),
    OccurrenceClassification(OccurrenceId, ClassificationDimensionId),
    Collection(CollectionId),
    Import(ImportId),
    Definition(DefinitionId),
    Feature(FeatureId),
    BodyFeatureSuppression(DefinitionId, BodyId),
    Occurrence(OccurrenceId),
    GroundedOccurrence(OccurrenceId),
    AssemblyMate(AssemblyMateId),
    AssemblyJoint(AssemblyJointId),
    AssemblyMotionCoupling(AssemblyMotionCouplingId),
    AssemblyMotionStudy(AssemblyMotionStudyId),
    MechanicalInterface(MechanicalInterfaceId),
    MechanicalCondition(MechanicalConditionId),
    DrawingSheet(DrawingSheetId),
    Group(GroupId),
    LocalGroup(LocalGroupKey),
    LocalOccurrence(LocalOccurrenceKey),
    DefinitionUsers(DefinitionId),
    FeatureUsers(FeatureId),
    FeatureParameterBindings(FeatureId),
    GroupChildren(GroupId),
    GroupSubtree(GroupId),
    OccurrenceCollections(OccurrenceId),
    /// Prunable part references whose nested path goes through this definition.
    PartReferencesThrough(DefinitionId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposalBudget {
    pub max_commands: usize,
    pub max_read_dependencies: usize,
    pub max_write_targets: usize,
}

impl ProposalBudget {
    pub const HOST_MAX: Self = Self {
        max_commands: 512,
        max_read_dependencies: 1_024,
        max_write_targets: 1_024,
    };

    /// A rule program is compiled into commands by Kečup itself, not proposed
    /// command by command, so one program (a whole assembly with every
    /// machined feature) gets a budget far above the per-edit [`Self::HOST_MAX`].
    pub const RULE_PROGRAM: Self = Self {
        max_commands: 200_000,
        max_read_dependencies: 200_000,
        max_write_targets: 200_000,
    };

    pub const M7A_SINGLE_CHANGE: Self = Self {
        max_commands: 1,
        max_read_dependencies: 64,
        max_write_targets: 1,
    };

    pub const M18C_CREATE_FEATURE: Self = Self {
        max_commands: 1,
        max_read_dependencies: 64,
        max_write_targets: 2,
    };

    pub const M18C_CLONE_PROFILE_DEFINITION: Self = Self {
        max_commands: 1,
        max_read_dependencies: 64,
        max_write_targets: 3,
    };

    pub const T18_ATOMIC_MULTI_COMMAND_EDIT: Self = Self {
        max_commands: 3,
        max_read_dependencies: 64,
        max_write_targets: 1,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProposalCost {
    pub commands: usize,
    pub read_dependencies: usize,
    pub write_targets: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalPrincipal {
    ManualClient,
    Human(u64),
    LocalAssistant,
    Plugin(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighRiskClass {
    DestructiveBulkChange,
    Overwrite,
    LossyConversion,
    ExternalDisclosure,
    ReleaseManufacturingExportWithWarnings,
    CapabilityExpansion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalRisk {
    Standard,
    High(HighRiskClass),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighRiskScope {
    pub(super) class: HighRiskClass,
    pub(super) destination: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) path: Option<String>,
}

impl HighRiskScope {
    pub fn new(
        class: HighRiskClass,
        destination: Option<String>,
        provider: Option<String>,
        path: Option<String>,
    ) -> Result<Self, HumanConfirmationError> {
        for value in [destination.as_deref(), provider.as_deref(), path.as_deref()]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
                return Err(HumanConfirmationError::InvalidScope);
            }
        }
        if matches!(class, HighRiskClass::ExternalDisclosure)
            && (destination.is_none() || provider.is_none())
        {
            return Err(HumanConfirmationError::InvalidScope);
        }
        if matches!(
            class,
            HighRiskClass::Overwrite
                | HighRiskClass::LossyConversion
                | HighRiskClass::ReleaseManufacturingExportWithWarnings
        ) && path.is_none()
        {
            return Err(HumanConfirmationError::InvalidScope);
        }
        Ok(Self {
            class,
            destination,
            provider,
            path,
        })
    }

    #[must_use]
    pub const fn class(&self) -> HighRiskClass {
        self.class
    }

    #[must_use]
    pub fn destination(&self) -> Option<&str> {
        self.destination.as_deref()
    }

    #[must_use]
    pub fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalConfirmation {
    ReviewRequired,
    HumanOnly(HighRiskScope),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalGoal {
    CanonicalPreview,
    CreateEvaluatorInput(NodeId),
    CreateEvaluatorExpression(NodeId),
    CreateEvaluatorRule(NodeId),
    CreateRuleOverride(u64),
    DeleteRuleOverride(u64),
    CreateFeatureParameterBinding(FeatureParameterTarget),
    DeleteFeatureParameterBinding(FeatureParameterTarget),
    CreatePersistentDimension(PersistentDimensionId),
    CreateSpace(SpaceId),
    CreateClearanceVolume(ClearanceVolumeId),
    CreateJoint(JointId),
    RecomputeFeatureParameter(FeatureParameterTarget),
    DeleteJoint(JointId),
    DeleteSpace(SpaceId),
    DeleteClearanceVolume(ClearanceVolumeId),
    DeletePersistentDimension(PersistentDimensionId),
    SetRuleDimension(NodeId),
    RenameEvaluatorNode(NodeId),
    SetEvaluatorExpression(NodeId),
    SetRuleOutputs(NodeId),
    SetFeatureDimension(FeatureId),
    SetProfilePoints(FeatureId),
    RenameDefinition(DefinitionId),
    SetOccurrenceVisibility(OccurrenceId),
    SetOccurrenceTranslation(OccurrenceId),
    AtomicMultiCommandEdit(OccurrenceId),
    SetOccurrenceTags(OccurrenceId),
    SetTagVisibility(TagId),
    RepointOccurrence(OccurrenceId),
    SetOccurrenceParent(OccurrenceId),
    SetGroupTranslation(GroupId),
    SetGroupParent(GroupId),
    SetCollectionOccurrences(CollectionId),
    CreateTag(TagId),
    DeleteTag(TagId),
    CreateCollection(CollectionId),
    DeleteCollection(CollectionId),
    DeleteGroup(GroupId),
    DeleteOccurrence(OccurrenceId),
    CreateDefinition(DefinitionId),
    DeleteDefinition(DefinitionId),
    CreateProfileFeature(FeatureId),
    DeleteProfileFeature(FeatureId),
    CreateGroup(GroupId),
    CreateOccurrence(OccurrenceId),
    CloneProfileDefinitionAndRepoint(OccurrenceId),
    ConvertEmptyGroupToComponent(GroupId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalAssumption {
    TargetExists(AuthoritativeDependency),
    TargetMissing(AuthoritativeDependency),
    TargetHasDimension(AuthoritativeDependency),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProposalValue {
    Missing,
    EvaluatorInputState {
        name: String,
        dimension: Dimension,
        dependencies: Vec<NodeId>,
    },
    EvaluatorExpressionState {
        name: String,
        expression: String,
        dependencies: Vec<NodeId>,
    },
    EvaluatorRuleState {
        name: String,
        expression: String,
        dependencies: Vec<NodeId>,
        input_ports: Vec<PortSpec>,
        output_ports: Vec<PortSpec>,
        outputs: Vec<RuleOutput>,
        override_parameters: Vec<OverrideParameterSpec>,
    },
    RuleOverrideState {
        target: DerivedIdentity,
        parameter: String,
        value: f64,
        health: SlotResolution,
    },
    FeatureParameterBindingState {
        target: FeatureParameterTarget,
        derived_from: DerivedIdentity,
    },
    JointState {
        participant_a: DerivedIdentity,
        participant_b: DerivedIdentity,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
    },
    SpaceState {
        purpose: String,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
        adjacent_to: Vec<SpaceId>,
        accessible_to: Vec<SpaceId>,
    },
    ClearanceVolumeState {
        owner: ClearanceOwner,
        reason: String,
        volume_min: [f64; 3],
        volume_max: [f64; 3],
        coordinate_frame: ClearanceCoordinateFrame,
        tolerance_mm: f64,
        severity: ClearanceSeverity,
        derived_from: Option<DerivedIdentity>,
    },
    PersistentDimensionState {
        name: String,
        target: PersistentDimensionTarget,
        presentation: DimensionPresentation,
    },
    Boolean(bool),
    Dimension(Dimension),
    RuleOutputs(Vec<RuleOutput>),
    ProfilePoints(Vec<[f64; 2]>),
    Transform(Transform),
    Tags(BTreeSet<TagId>),
    TagState {
        name: String,
        visible: bool,
    },
    CollectionState {
        name: String,
        occurrence_ids: Vec<OccurrenceId>,
    },
    Definition(DefinitionId),
    DefinitionState {
        name: String,
        feature_ids: Vec<FeatureId>,
        local_occurrence_ids: Vec<LocalOccurrenceId>,
        local_group_ids: Vec<LocalGroupId>,
    },
    DefinitionFeatures(Vec<FeatureId>),
    ProfileFeatureState {
        definition: DefinitionId,
        name: String,
        points_mm: Vec<[f64; 2]>,
    },
    Group(Option<GroupId>),
    GroupState {
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
    },
    OccurrenceState {
        definition: DefinitionId,
        name: String,
        transform: Transform,
        parent: Option<GroupId>,
        tags: BTreeSet<TagId>,
        visible: bool,
    },
    Occurrences(Vec<OccurrenceId>),
    Text(String),
    Digest(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposalDiffEntry {
    pub target: AuthoritativeDependency,
    pub before: ProposalValue,
    pub after: ProposalValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalContext {
    pub principal: ProposalPrincipal,
    pub goal: ProposalGoal,
    pub assumptions: Vec<ProposalAssumption>,
    pub risk: ProposalRisk,
    pub confirmation: ProposalConfirmation,
    pub requested_budget: ProposalBudget,
}

impl ProposalContext {
    #[must_use]
    pub fn local_assistant_model() -> Self {
        Self {
            principal: ProposalPrincipal::LocalAssistant,
            goal: ProposalGoal::CanonicalPreview,
            assumptions: Vec::new(),
            risk: ProposalRisk::Standard,
            confirmation: ProposalConfirmation::ReviewRequired,
            requested_budget: ProposalBudget::HOST_MAX,
        }
    }

    /// The assistant publishing a compiled rule program.
    #[must_use]
    pub fn rule_program() -> Self {
        Self {
            requested_budget: ProposalBudget::RULE_PROGRAM,
            ..Self::local_assistant_model()
        }
    }

    #[must_use]
    pub fn canonical_preview() -> Self {
        Self {
            principal: ProposalPrincipal::ManualClient,
            goal: ProposalGoal::CanonicalPreview,
            assumptions: Vec::new(),
            risk: ProposalRisk::Standard,
            confirmation: ProposalConfirmation::ReviewRequired,
            requested_budget: ProposalBudget::HOST_MAX,
        }
    }
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
    pub(super) schema: &'static str,
    pub(super) commands: Vec<CanonicalCommand>,
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
    pub fn edit_evaluator_and_recompute_affected(
        edit: EvaluatorParameterEdit,
        identity: EvaluationIdentity,
    ) -> Self {
        let scope = FeatureParameterRecomputeScope::affected_by(edit.affected_node());
        Self::new(vec![
            edit.into_command(),
            CanonicalCommand::RecomputeFeatureParameters { identity, scope },
        ])
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
        digest.value(&self.commands);
        digest.finish()
    }
}
