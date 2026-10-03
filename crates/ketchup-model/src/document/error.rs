use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalError {
    EmptySourceToken,
    InvalidDecimalToken,
    DimensionOutsideEnvelope,
    InvalidRevolve,
    InvalidPlanarOffset,
    InvalidSweep,
    InvalidWeldmentMember,
    InvalidWeldmentJoint,
    InvalidLoft,
    InvalidSheetMetal(SheetMetalError),
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
    InvalidConstructionGeometry,
    Sketch(SketchError),
    InvalidStableSubshapeRole,
    SubshapeRolesNotCanonical,
    InvalidTopologicalFeatureReference,
    InvalidMeshBody,
    DefinitionAlreadyExists(DefinitionId),
    DefinitionNotFound(DefinitionId),
    DefinitionInUse(DefinitionId),
    DefinitionNotEmpty(DefinitionId),
    BodyAlreadyExists(DefinitionId, BodyId),
    BodyNotFound(DefinitionId, BodyId),
    BodyInUse(DefinitionId, BodyId),
    BodyIsActive(DefinitionId, BodyId),
    BodyInputsNotCanonical,
    InvalidBodyCommand,
    InvalidBodyAuthoringPlan,
    InvalidBodyContract,
    InvalidBodyOwnership(FeatureId),
    BodyDependencyCycle(DefinitionId),
    UnresolvedBodyOwnershipReference(FeatureId),
    FeatureAlreadyExists(FeatureId),
    FeatureNotFound(FeatureId),
    FeatureHasNoDimension(FeatureId),
    FeatureIsNotProfile(FeatureId),
    FeatureDependencyCycle(FeatureId),
    InvalidFeatureSuppression(DefinitionId, BodyId),
    FeatureSuppressionUnchanged(DefinitionId, BodyId),
    InvalidFeatureParameterBinding(FeatureParameterTarget),
    FeatureParameterBindingNotFound(FeatureParameterTarget),
    OccurrenceAlreadyExists(OccurrenceId),
    OccurrenceNotFound(OccurrenceId),
    OccurrenceInAssemblyMate(OccurrenceId),
    OccurrenceInAssemblyJoint(OccurrenceId),
    OccurrenceInPinJoint(OccurrenceId),
    PinJointNotFound(PinJointId),
    PinJoint(PinJointError),
    AssemblyRecipeNotFound,
    AssemblyRecipe(AssemblyRecipeError),
    AssemblyMateAlreadyExists(AssemblyMateId),
    AssemblyMateNotFound(AssemblyMateId),
    InvalidAssemblyMate(AssemblyMateId),
    AssemblyJointAlreadyExists(AssemblyJointId),
    AssemblyJointNotFound(AssemblyJointId),
    AssemblyJointInMotionStudy(AssemblyJointId),
    AssemblyJointInMotionCoupling(AssemblyJointId),
    InvalidAssemblyJoint(AssemblyJointId),
    AssemblyMotionCouplingAlreadyExists(AssemblyMotionCouplingId),
    AssemblyMotionCouplingNotFound(AssemblyMotionCouplingId),
    InvalidAssemblyMotionCoupling(AssemblyMotionCouplingId),
    MechanicalInterfaceAlreadyExists(MechanicalInterfaceId),
    MechanicalInterfaceNotFound(MechanicalInterfaceId),
    MechanicalInterfaceInCondition(MechanicalInterfaceId),
    InvalidMechanicalInterface(MechanicalInterfaceId),
    MechanicalConditionAlreadyExists(MechanicalConditionId),
    MechanicalConditionNotFound(MechanicalConditionId),
    InvalidMechanicalCondition(MechanicalConditionId),
    UnsynchronizedAssemblyJointPosition(AssemblyJointId),
    AssemblyMotionStudyAlreadyExists(AssemblyMotionStudyId),
    AssemblyMotionStudyNotFound(AssemblyMotionStudyId),
    InvalidAssemblyMotionStudy(AssemblyMotionStudyId),
    StaleAssemblySolve,
    InvalidAssemblySolvePublication,
    UnsolvedAssemblySolvePublication(Box<crate::assembly_joint::AssemblyKinematicSolveError>),
    DrawingSheetAlreadyExists(DrawingSheetId),
    DrawingSheetNotFound(DrawingSheetId),
    Drawing(DrawingError),
    GroupAlreadyExists(GroupId),
    GroupNotFound(GroupId),
    GroupNotEmpty(GroupId),
    GroupCycle(GroupId),
    InvalidFeatureOwnership(FeatureId),
    InvalidFeatureMap,
    InvalidSolidToolPlan,
    UnsupportedSolidToolTransform,
    OccurrenceDefinitionMismatch,
    InvalidLocalGraph,
    LocalOccurrenceNotFound(LocalOccurrenceKey),
    LocalOccurrenceAlreadyExists(LocalOccurrenceKey),
    LocalGroupNotFound(LocalGroupKey),
    LocalGroupAlreadyExists(LocalGroupKey),
    InvalidInstancePath,
    InvalidProductionCode,
    DuplicateProductionCode(String),
    InvalidProductionCodePath(InstancePath),
    IdExhausted,
    WrongNodeKind(NodeId),
    OverrideAlreadyExists(u64),
    OverrideNotFound(u64),
    JointAlreadyExists(JointId),
    JointNotFound(JointId),
    SpaceAlreadyExists(SpaceId),
    SpaceNotFound(SpaceId),
    ClearanceVolumeAlreadyExists(ClearanceVolumeId),
    ClearanceVolumeNotFound(ClearanceVolumeId),
    CamPlanNotFound(CamPlanId),
    Cam(CamError),
    PersistentDimensionNotFound(PersistentDimensionId),
    PersistentDimensionAlreadyExists(PersistentDimensionId),
    TagAlreadyExists(TagId),
    TagNotFound(TagId),
    TagInUse(TagId),
    InvalidClassificationDimension(ClassificationDimensionId),
    ClassificationDimensionNotFound(ClassificationDimensionId),
    ClassificationCategoryNotFound(ClassificationDimensionId, ClassificationCategoryId),
    ClassificationCategoryInUse(ClassificationDimensionId),
    CollectionAlreadyExists(CollectionId),
    CollectionNotFound(CollectionId),
    CollectionMembershipNotCanonical(CollectionId),
    OccurrenceInCollection(OccurrenceId),
    InvalidImportReceipt(ImportContractError),
    ImportAlreadyExists(ImportId),
    InvalidPersistentDimensionTarget,
    InvalidDimensionPresentation,
    UndeclaredOverrideParameter,
    UnresolvedDerivedOutput,
    EvaluationEnvelopeMismatch,
    EvaluationEvidenceMismatch,
    FailedEvaluation(NodeId),
    RevisionExhausted,
    RuleProgram(RuleProgramError),
    Graph(GraphError),
    Prismatic(PrismaticError),
    Space(SpaceError),
    /// `error` raised because of a lower-level failure named by `cause`.
    Caused {
        error: Box<CanonicalError>,
        cause: String,
    },
}

impl CanonicalError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EmptySourceToken => "canonical.empty_source_token",
            Self::InvalidDecimalToken => "canonical.invalid_decimal_token",
            Self::DimensionOutsideEnvelope => "canonical.dimension_outside_envelope",
            Self::InvalidRevolve => "canonical.invalid_revolve",
            Self::InvalidPlanarOffset => "canonical.invalid_planar_offset",
            Self::InvalidSweep => "canonical.invalid_sweep",
            Self::InvalidWeldmentMember => "canonical.invalid_weldment_member",
            Self::InvalidWeldmentJoint => "canonical.invalid_weldment_joint",
            Self::InvalidLoft => "canonical.invalid_loft",
            Self::InvalidSheetMetal(_) => "canonical.invalid_sheet_metal",
            Self::ReservedNodeId => "canonical.reserved_node_id",
            Self::EmptyNodeName => "canonical.empty_node_name",
            Self::DependenciesNotCanonical => "canonical.dependencies_not_canonical",
            Self::DependencyCycle(..) => "canonical.dependency_cycle",
            Self::NodeAlreadyExists(..) => "canonical.node_already_exists",
            Self::NodeNotFound(..) => "canonical.node_not_found",
            Self::MissingDependency(..) => "canonical.missing_dependency",
            Self::UnsupportedCommandSchema => "canonical.unsupported_command_schema",
            Self::EmptyCommandBatch => "canonical.empty_command_batch",
            Self::ReservedProductId => "canonical.reserved_product_id",
            Self::EmptyProductName => "canonical.empty_product_name",
            Self::InvalidTransform => "canonical.invalid_transform",
            Self::InvalidProfile => "canonical.invalid_profile",
            Self::InvalidConstructionGeometry => "canonical.invalid_construction_geometry",
            Self::Sketch(..) => "canonical.sketch",
            Self::InvalidStableSubshapeRole => "canonical.invalid_stable_subshape_role",
            Self::SubshapeRolesNotCanonical => "canonical.subshape_roles_not_canonical",
            Self::InvalidTopologicalFeatureReference => {
                "canonical.invalid_topological_feature_reference"
            }
            Self::InvalidMeshBody => "canonical.invalid_mesh_body",
            Self::DefinitionAlreadyExists(..) => "canonical.definition_already_exists",
            Self::DefinitionNotFound(..) => "canonical.definition_not_found",
            Self::DefinitionInUse(..) => "canonical.definition_in_use",
            Self::DefinitionNotEmpty(..) => "canonical.definition_not_empty",
            Self::BodyAlreadyExists(..) => "canonical.body_already_exists",
            Self::BodyNotFound(..) => "canonical.body_not_found",
            Self::BodyInUse(..) => "canonical.body_in_use",
            Self::BodyIsActive(..) => "canonical.body_is_active",
            Self::BodyInputsNotCanonical => "canonical.body_inputs_not_canonical",
            Self::InvalidBodyCommand => "canonical.invalid_body_command",
            Self::InvalidBodyAuthoringPlan => "canonical.invalid_body_authoring_plan",
            Self::InvalidBodyContract => "canonical.invalid_body_contract",
            Self::InvalidBodyOwnership(..) => "canonical.invalid_body_ownership",
            Self::BodyDependencyCycle(..) => "canonical.body_dependency_cycle",
            Self::UnresolvedBodyOwnershipReference(..) => {
                "canonical.unresolved_body_ownership_reference"
            }
            Self::FeatureAlreadyExists(..) => "canonical.feature_already_exists",
            Self::FeatureNotFound(..) => "canonical.feature_not_found",
            Self::FeatureHasNoDimension(..) => "canonical.feature_has_no_dimension",
            Self::FeatureIsNotProfile(..) => "canonical.feature_is_not_profile",
            Self::FeatureDependencyCycle(..) => "canonical.feature_dependency_cycle",
            Self::InvalidFeatureSuppression(..) => "canonical.invalid_feature_suppression",
            Self::FeatureSuppressionUnchanged(..) => "canonical.feature_suppression_unchanged",
            Self::InvalidFeatureParameterBinding(..) => {
                "canonical.invalid_feature_parameter_binding"
            }
            Self::FeatureParameterBindingNotFound(..) => {
                "canonical.feature_parameter_binding_not_found"
            }
            Self::OccurrenceAlreadyExists(..) => "canonical.occurrence_already_exists",
            Self::OccurrenceNotFound(..) => "canonical.occurrence_not_found",
            Self::OccurrenceInAssemblyMate(..) => "canonical.occurrence_in_assembly_mate",
            Self::OccurrenceInAssemblyJoint(..) => "canonical.occurrence_in_assembly_joint",
            Self::OccurrenceInPinJoint(..) => "canonical.occurrence_in_pin_joint",
            Self::PinJointNotFound(..) => "canonical.pin_joint_not_found",
            Self::PinJoint(..) => "canonical.pin_joint",
            Self::AssemblyRecipeNotFound => "canonical.assembly_recipe_not_found",
            Self::AssemblyRecipe(..) => "canonical.assembly_recipe",
            Self::AssemblyMateAlreadyExists(..) => "canonical.assembly_mate_already_exists",
            Self::AssemblyMateNotFound(..) => "canonical.assembly_mate_not_found",
            Self::InvalidAssemblyMate(..) => "canonical.invalid_assembly_mate",
            Self::AssemblyJointAlreadyExists(..) => "canonical.assembly_joint_already_exists",
            Self::AssemblyJointNotFound(..) => "canonical.assembly_joint_not_found",
            Self::AssemblyJointInMotionStudy(..) => "canonical.assembly_joint_in_motion_study",
            Self::AssemblyJointInMotionCoupling(..) => {
                "canonical.assembly_joint_in_motion_coupling"
            }
            Self::InvalidAssemblyJoint(..) => "canonical.invalid_assembly_joint",
            Self::AssemblyMotionCouplingAlreadyExists(..) => {
                "canonical.assembly_motion_coupling_already_exists"
            }
            Self::AssemblyMotionCouplingNotFound(..) => {
                "canonical.assembly_motion_coupling_not_found"
            }
            Self::InvalidAssemblyMotionCoupling(..) => "canonical.invalid_assembly_motion_coupling",
            Self::MechanicalInterfaceAlreadyExists(..) => {
                "canonical.mechanical_interface_already_exists"
            }
            Self::MechanicalInterfaceNotFound(..) => "canonical.mechanical_interface_not_found",
            Self::MechanicalInterfaceInCondition(..) => {
                "canonical.mechanical_interface_in_condition"
            }
            Self::InvalidMechanicalInterface(..) => "canonical.invalid_mechanical_interface",
            Self::MechanicalConditionAlreadyExists(..) => {
                "canonical.mechanical_condition_already_exists"
            }
            Self::MechanicalConditionNotFound(..) => "canonical.mechanical_condition_not_found",
            Self::InvalidMechanicalCondition(..) => "canonical.invalid_mechanical_condition",
            Self::UnsynchronizedAssemblyJointPosition(..) => {
                "canonical.unsynchronized_assembly_joint_position"
            }
            Self::AssemblyMotionStudyAlreadyExists(..) => {
                "canonical.assembly_motion_study_already_exists"
            }
            Self::AssemblyMotionStudyNotFound(..) => "canonical.assembly_motion_study_not_found",
            Self::InvalidAssemblyMotionStudy(..) => "canonical.invalid_assembly_motion_study",
            Self::StaleAssemblySolve => "canonical.stale_assembly_solve",
            Self::InvalidAssemblySolvePublication => "canonical.invalid_assembly_solve_publication",
            Self::UnsolvedAssemblySolvePublication(_) => {
                "canonical.unsolved_assembly_solve_publication"
            }
            Self::DrawingSheetAlreadyExists(..) => "canonical.drawing_sheet_already_exists",
            Self::DrawingSheetNotFound(..) => "canonical.drawing_sheet_not_found",
            Self::Drawing(..) => "canonical.drawing",
            Self::GroupAlreadyExists(..) => "canonical.group_already_exists",
            Self::GroupNotFound(..) => "canonical.group_not_found",
            Self::GroupNotEmpty(..) => "canonical.group_not_empty",
            Self::GroupCycle(..) => "canonical.group_cycle",
            Self::InvalidFeatureOwnership(..) => "canonical.invalid_feature_ownership",
            Self::InvalidFeatureMap => "canonical.invalid_feature_map",
            Self::InvalidSolidToolPlan => "canonical.invalid_solid_tool_plan",
            Self::UnsupportedSolidToolTransform => "canonical.unsupported_solid_tool_transform",
            Self::OccurrenceDefinitionMismatch => "canonical.occurrence_definition_mismatch",
            Self::InvalidLocalGraph => "canonical.invalid_local_graph",
            Self::LocalOccurrenceNotFound(..) => "canonical.local_occurrence_not_found",
            Self::LocalOccurrenceAlreadyExists(..) => "canonical.local_occurrence_already_exists",
            Self::LocalGroupNotFound(..) => "canonical.local_group_not_found",
            Self::LocalGroupAlreadyExists(..) => "canonical.local_group_already_exists",
            Self::InvalidInstancePath => "canonical.invalid_instance_path",
            Self::InvalidProductionCode => "canonical.invalid_production_code",
            Self::DuplicateProductionCode(_) => "canonical.duplicate_production_code",
            Self::InvalidProductionCodePath(_) => "canonical.invalid_production_code_path",
            Self::IdExhausted => "canonical.id_exhausted",
            Self::WrongNodeKind(..) => "canonical.wrong_node_kind",
            Self::OverrideAlreadyExists(..) => "canonical.override_already_exists",
            Self::OverrideNotFound(..) => "canonical.override_not_found",
            Self::JointAlreadyExists(..) => "canonical.joint_already_exists",
            Self::JointNotFound(..) => "canonical.joint_not_found",
            Self::SpaceAlreadyExists(..) => "canonical.space_already_exists",
            Self::SpaceNotFound(..) => "canonical.space_not_found",
            Self::ClearanceVolumeAlreadyExists(..) => "canonical.clearance_volume_already_exists",
            Self::ClearanceVolumeNotFound(..) => "canonical.clearance_volume_not_found",
            Self::CamPlanNotFound(..) => "canonical.cam_plan_not_found",
            Self::Cam(..) => "canonical.cam",
            Self::PersistentDimensionNotFound(..) => "canonical.persistent_dimension_not_found",
            Self::PersistentDimensionAlreadyExists(..) => {
                "canonical.persistent_dimension_already_exists"
            }
            Self::TagAlreadyExists(..) => "canonical.tag_already_exists",
            Self::TagNotFound(..) => "canonical.tag_not_found",
            Self::TagInUse(..) => "canonical.tag_in_use",
            Self::InvalidClassificationDimension(..) => {
                "canonical.invalid_classification_dimension"
            }
            Self::ClassificationDimensionNotFound(..) => {
                "canonical.classification_dimension_not_found"
            }
            Self::ClassificationCategoryNotFound(..) => {
                "canonical.classification_category_not_found"
            }
            Self::ClassificationCategoryInUse(..) => "canonical.classification_category_in_use",
            Self::CollectionAlreadyExists(..) => "canonical.collection_already_exists",
            Self::CollectionNotFound(..) => "canonical.collection_not_found",
            Self::CollectionMembershipNotCanonical(..) => {
                "canonical.collection_membership_not_canonical"
            }
            Self::OccurrenceInCollection(..) => "canonical.occurrence_in_collection",
            Self::InvalidImportReceipt(_) => "canonical.invalid_import_receipt",
            Self::ImportAlreadyExists(..) => "canonical.import_already_exists",
            Self::InvalidPersistentDimensionTarget => {
                "canonical.invalid_persistent_dimension_target"
            }
            Self::InvalidDimensionPresentation => "canonical.invalid_dimension_presentation",
            Self::UndeclaredOverrideParameter => "canonical.undeclared_override_parameter",
            Self::UnresolvedDerivedOutput => "canonical.unresolved_derived_output",
            Self::EvaluationEnvelopeMismatch => "canonical.evaluation_envelope_mismatch",
            Self::EvaluationEvidenceMismatch => "canonical.evaluation_evidence_mismatch",
            Self::FailedEvaluation(..) => "canonical.failed_evaluation",
            Self::RevisionExhausted => "canonical.revision_exhausted",
            Self::RuleProgram(_) => "canonical.rule_program",
            Self::Graph(..) => "canonical.graph",
            Self::Prismatic(..) => "canonical.prismatic",
            Self::Space(..) => "canonical.space",
            Self::Caused { error, .. } => error.code(),
        }
    }

    /// Wraps this error with the lower-level failure that caused it; `code` is unchanged.
    #[must_use]
    pub fn because(self, cause: impl fmt::Display) -> Self {
        Self::Caused {
            error: Box::new(self),
            cause: cause.to_string(),
        }
    }
}

impl From<GraphError> for CanonicalError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<ketchup_geometry::slot::SlotError> for CanonicalError {
    fn from(error: ketchup_geometry::slot::SlotError) -> Self {
        Self::Graph(error.into())
    }
}

impl From<DimensionError> for CanonicalError {
    fn from(error: DimensionError) -> Self {
        match error {
            DimensionError::EmptySourceToken => Self::EmptySourceToken,
            DimensionError::InvalidDecimalToken => Self::InvalidDecimalToken,
            DimensionError::OutsideEnvelope => Self::DimensionOutsideEnvelope,
        }
    }
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySourceToken => write!(formatter, "{}", DimensionError::EmptySourceToken),
            Self::InvalidDecimalToken => write!(formatter, "{}", DimensionError::InvalidDecimalToken),
            Self::DimensionOutsideEnvelope => write!(formatter, "{}", DimensionError::OutsideEnvelope),
            Self::InvalidRevolve => formatter.write_str("revolve axis or angle is invalid"),
            Self::InvalidPlanarOffset => {
                formatter.write_str("planar offset distance or bounded profile is invalid")
            }
            Self::InvalidSweep => {
                formatter.write_str("sweep requires one bounded closed profile and a compatible non-degenerate open path")
            }
            Self::InvalidWeldmentMember => formatter.write_str(
                "weldment member requires a bounded closed profile, a canonical spatial path, and orientation in [-180, 180) degrees",
            ),
            Self::InvalidWeldmentJoint => formatter.write_str(
                "weldment joint requires two distinct straight members meeting at one manufacturable nonparallel endpoint",
            ),
            Self::InvalidLoft => {
                formatter.write_str("loft requires ordered bounded closed-profile sections")
            }
            Self::InvalidSheetMetal(error) => write!(formatter, "invalid sheet-metal feature: {error}"),
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
            Self::InvalidConstructionGeometry => formatter
                .write_str("construction geometry must contain finite bounded coordinates"),
            Self::Sketch(error) => write!(formatter, "invalid workplane or sketch: {error}"),
            Self::InvalidStableSubshapeRole => formatter.write_str(
                "stable subshape role must be a bounded canonical semantic identifier",
            ),
            Self::SubshapeRolesNotCanonical => formatter.write_str(
                "stable subshape roles must be non-empty, unique, and strictly sorted",
            ),
            Self::InvalidTopologicalFeatureReference => formatter.write_str(
                "topological feature references must be valid, kind-correct, unique, and strictly sorted",
            ),
            Self::InvalidMeshBody => formatter.write_str(
                "mesh body must be finite, closed, consistently oriented, non-degenerate, and carry valid authority provenance",
            ),
            Self::DefinitionAlreadyExists(id) => {
                write!(formatter, "definition {} already exists", id.0)
            }
            Self::DefinitionNotFound(id) => write!(formatter, "definition {} does not exist", id.0),
            Self::DefinitionInUse(id) => write!(formatter, "definition {} is still used", id.0),
            Self::DefinitionNotEmpty(id) => write!(formatter, "definition {} is not empty", id.0),
            Self::BodyAlreadyExists(definition, body) => write!(
                formatter,
                "body {} already exists in definition {}",
                body.0, definition.0
            ),
            Self::BodyNotFound(definition, body) => write!(
                formatter,
                "body {} does not exist in definition {}",
                body.0, definition.0
            ),
            Self::BodyInUse(definition, body) => write!(
                formatter,
                "body {} in definition {} is still used",
                body.0, definition.0
            ),
            Self::BodyIsActive(definition, body) => write!(
                formatter,
                "body {} in definition {} is active",
                body.0, definition.0
            ),
            Self::BodyInputsNotCanonical => formatter
                .write_str("feature input bodies must be unique and strictly sorted"),
            Self::InvalidBodyCommand => formatter.write_str("command is not a body mutation"),
            Self::InvalidBodyAuthoringPlan => formatter.write_str(
                "multi-body authoring requires distinct bodies with resolved terminal Pad or Extrusion outputs",
            ),
            Self::InvalidBodyContract => {
                formatter.write_str("definition body contract is incomplete or non-canonical")
            }
            Self::InvalidBodyOwnership(id) => {
                write!(formatter, "feature {} has invalid body ownership", id.0)
            }
            Self::BodyDependencyCycle(id) => {
                write!(formatter, "body dependency cycle in definition {}", id.0)
            }
            Self::UnresolvedBodyOwnershipReference(id) => write!(
                formatter,
                "feature {} body ownership depends on an ambiguous or lost reference",
                id.0
            ),
            Self::FeatureAlreadyExists(id) => write!(formatter, "feature {} already exists", id.0),
            Self::FeatureNotFound(id) => write!(formatter, "feature {} does not exist", id.0),
            Self::FeatureHasNoDimension(id) => {
                write!(formatter, "feature {} has no editable dimension", id.0)
            }
            Self::FeatureIsNotProfile(id) => {
                write!(formatter, "feature {} is not a profile", id.0)
            }
            Self::FeatureDependencyCycle(id) => {
                write!(formatter, "feature dependency cycle at {}", id.0)
            }
            Self::InvalidFeatureSuppression(definition, body) => write!(
                formatter,
                "feature suppression for body {} in definition {} is not a dependency-closed suffix",
                body.0, definition.0
            ),
            Self::FeatureSuppressionUnchanged(definition, body) => write!(
                formatter,
                "feature suppression for body {} in definition {} is unchanged",
                body.0, definition.0
            ),
            Self::InvalidFeatureParameterBinding(target) => write!(
                formatter,
                "feature {} parameter {} has an invalid derived binding",
                target.feature_id.0,
                target.path.as_str()
            ),
            Self::FeatureParameterBindingNotFound(target) => write!(
                formatter,
                "feature {} parameter {} has no derived binding",
                target.feature_id.0,
                target.path.as_str()
            ),
            Self::OccurrenceAlreadyExists(id) => {
                write!(formatter, "occurrence {} already exists", id.0)
            }
            Self::OccurrenceNotFound(id) => write!(formatter, "occurrence {} does not exist", id.0),
            Self::OccurrenceInAssemblyMate(id) => {
                write!(formatter, "occurrence {} is still used by an assembly mate", id.0)
            }
            Self::OccurrenceInAssemblyJoint(id) => {
                write!(formatter, "occurrence {} is still used by an assembly joint", id.0)
            }
            Self::OccurrenceInPinJoint(id) => {
                write!(formatter, "occurrence {} is still used by a pin joint", id.0)
            }
            Self::PinJointNotFound(id) => {
                write!(formatter, "pin joint {} does not exist", id.0)
            }
            Self::PinJoint(error) => write!(formatter, "invalid pin joint: {error}"),
            Self::AssemblyRecipeNotFound => formatter.write_str("assembly recipe does not exist"),
            Self::AssemblyRecipe(error) => write!(formatter, "{error}"),
            Self::AssemblyMateAlreadyExists(id) => {
                write!(formatter, "assembly mate {} already exists", id.0)
            }
            Self::AssemblyMateNotFound(id) => {
                write!(formatter, "assembly mate {} does not exist", id.0)
            }
            Self::InvalidAssemblyMate(id) => {
                write!(formatter, "assembly mate {} is invalid or unresolved", id.0)
            }
            Self::AssemblyJointAlreadyExists(id) => {
                write!(formatter, "assembly joint {} already exists", id.0)
            }
            Self::AssemblyJointNotFound(id) => {
                write!(formatter, "assembly joint {} does not exist", id.0)
            }
            Self::AssemblyJointInMotionStudy(id) => {
                write!(formatter, "assembly joint {} is still used by a motion study", id.0)
            }
            Self::AssemblyJointInMotionCoupling(id) => write!(
                formatter,
                "assembly joint {} is still used by a motion coupling",
                id.0
            ),
            Self::InvalidAssemblyJoint(id) => {
                write!(formatter, "assembly joint {} is invalid", id.0)
            }
            Self::UnsynchronizedAssemblyJointPosition(id) => write!(
                formatter,
                "assembly joint {} position requires an atomic solve transform",
                id.0
            ),
            Self::AssemblyMotionCouplingAlreadyExists(id) => {
                write!(formatter, "assembly motion coupling {} already exists", id.0)
            }
            Self::AssemblyMotionCouplingNotFound(id) => {
                write!(formatter, "assembly motion coupling {} does not exist", id.0)
            }
            Self::InvalidAssemblyMotionCoupling(id) => {
                write!(formatter, "assembly motion coupling {} is invalid", id.0)
            }
            Self::MechanicalInterfaceAlreadyExists(id) => {
                write!(formatter, "mechanical interface {} already exists", id.0)
            }
            Self::MechanicalInterfaceNotFound(id) => {
                write!(formatter, "mechanical interface {} does not exist", id.0)
            }
            Self::MechanicalInterfaceInCondition(id) => write!(
                formatter,
                "mechanical interface {} is still used by a mechanical condition",
                id.0
            ),
            Self::InvalidMechanicalInterface(id) => {
                write!(formatter, "mechanical interface {} is invalid", id.0)
            }
            Self::MechanicalConditionAlreadyExists(id) => {
                write!(formatter, "mechanical condition {} already exists", id.0)
            }
            Self::MechanicalConditionNotFound(id) => {
                write!(formatter, "mechanical condition {} does not exist", id.0)
            }
            Self::InvalidMechanicalCondition(id) => {
                write!(formatter, "mechanical condition {} is invalid", id.0)
            }
            Self::AssemblyMotionStudyAlreadyExists(id) => {
                write!(formatter, "assembly motion study {} already exists", id.0)
            }
            Self::AssemblyMotionStudyNotFound(id) => {
                write!(formatter, "assembly motion study {} does not exist", id.0)
            }
            Self::InvalidAssemblyMotionStudy(id) => {
                write!(formatter, "assembly motion study {} is invalid", id.0)
            }
            Self::StaleAssemblySolve => {
                formatter.write_str("assembly solve source revision or digest is stale")
            }
            Self::InvalidAssemblySolvePublication => {
                formatter.write_str("assembly solve publication is empty, non-canonical, or grounded")
            }
            Self::UnsolvedAssemblySolvePublication(error) => {
                write!(formatter, "assembly solve publication cannot be re-solved: {error}")
            }
            Self::DrawingSheetAlreadyExists(id) => {
                write!(formatter, "drawing sheet {} already exists", id.0)
            }
            Self::DrawingSheetNotFound(id) => {
                write!(formatter, "drawing sheet {} does not exist", id.0)
            }
            Self::Drawing(error) => write!(formatter, "invalid drawing sheet: {error}"),
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
            Self::InvalidSolidToolPlan => {
                formatter.write_str("solid tool plan is incomplete or non-canonical")
            }
            Self::UnsupportedSolidToolTransform => formatter.write_str(
                "solid tools currently require root occurrences with translation-only transforms on the same extrusion plane",
            ),
            Self::OccurrenceDefinitionMismatch => {
                formatter.write_str("occurrence does not reference the requested source definition")
            }
            Self::InvalidLocalGraph => formatter.write_str("definition-local graph is invalid"),
            Self::LocalOccurrenceNotFound(key) => write!(formatter, "local occurrence {} in definition {} was not found; select an existing component member", key.local_id.0, key.definition_id.0),
            Self::LocalOccurrenceAlreadyExists(key) => write!(formatter, "local ID {} in definition {} is already used by a member or group; choose an unused local ID", key.local_id.0, key.definition_id.0),
            Self::LocalGroupNotFound(key) => write!(formatter, "local group {} in definition {} was not found; select an existing component group", key.local_id.0, key.definition_id.0),
            Self::LocalGroupAlreadyExists(key) => write!(formatter, "local ID {} in definition {} is already used by a member or group; choose an unused local ID", key.local_id.0, key.definition_id.0),
            Self::InvalidInstancePath => formatter.write_str("instance path is unresolved"),
            Self::InvalidProductionCode => formatter.write_str("production code must be 1..64 bytes of A-Z, 0-9, underscore or hyphen"),
            Self::DuplicateProductionCode(code) => write!(formatter, "production code {code} is already assigned in this document"),
            Self::InvalidProductionCodePath(path) => write!(formatter, "production code path {path:?} is not a physical instance or has been rebound; clear its code first"),
            Self::IdExhausted => formatter.write_str("canonical ID space is exhausted"),
            Self::WrongNodeKind(id) => write!(formatter, "node {} has the wrong kind", id.0),
            Self::OverrideAlreadyExists(id) => write!(formatter, "override {id} already exists"),
            Self::OverrideNotFound(id) => write!(formatter, "override {id} does not exist"),
            Self::JointAlreadyExists(id) => write!(formatter, "joint {} already exists", id.0),
            Self::JointNotFound(id) => write!(formatter, "joint {} does not exist", id.0),
            Self::SpaceAlreadyExists(id) => write!(formatter, "space {} already exists", id.0),
            Self::SpaceNotFound(id) => write!(formatter, "space {} does not exist", id.0),
            Self::ClearanceVolumeAlreadyExists(id) => {
                write!(formatter, "clearance volume {} already exists", id.0)
            }
            Self::ClearanceVolumeNotFound(id) => {
                write!(formatter, "clearance volume {} does not exist", id.0)
            }
            Self::CamPlanNotFound(id) => {
                write!(formatter, "CAM plan {} does not exist", id.0)
            }
            Self::Cam(error) => write!(formatter, "invalid CAM plan: {error}"),
            Self::PersistentDimensionNotFound(id) => {
                write!(formatter, "persistent dimension {} does not exist", id.0)
            }
            Self::PersistentDimensionAlreadyExists(id) => {
                write!(formatter, "persistent dimension {} already exists", id.0)
            }
            Self::TagAlreadyExists(id) => write!(formatter, "tag {} already exists", id.0),
            Self::TagNotFound(id) => write!(formatter, "tag {} does not exist", id.0),
            Self::TagInUse(id) => write!(formatter, "tag {} is still assigned", id.0),
            Self::InvalidClassificationDimension(id) => {
                write!(formatter, "classification dimension {} is invalid", id.0)
            }
            Self::ClassificationDimensionNotFound(id) => {
                write!(formatter, "classification dimension {} does not exist", id.0)
            }
            Self::ClassificationCategoryNotFound(dimension_id, category_id) => write!(
                formatter,
                "classification category {} does not exist in dimension {}",
                category_id.0, dimension_id.0
            ),
            Self::ClassificationCategoryInUse(id) => write!(
                formatter,
                "classification dimension {} would remove an assigned category",
                id.0
            ),
            Self::CollectionAlreadyExists(id) => {
                write!(formatter, "collection {} already exists", id.0)
            }
            Self::CollectionNotFound(id) => {
                write!(formatter, "collection {} does not exist", id.0)
            }
            Self::CollectionMembershipNotCanonical(id) => write!(
                formatter,
                "collection {} membership must be unique and strictly sorted",
                id.0
            ),
            Self::OccurrenceInCollection(id) => {
                write!(formatter, "occurrence {} is still in a collection", id.0)
            }
            Self::InvalidImportReceipt(error) => write!(formatter, "import receipt is invalid: {error}"),
            Self::ImportAlreadyExists(id) => {
                write!(formatter, "import {} already exists", id.0)
            }
            Self::InvalidPersistentDimensionTarget => {
                formatter.write_str("persistent dimension target is invalid")
            }
            Self::InvalidDimensionPresentation => {
                formatter.write_str("persistent dimension presentation is invalid")
            }
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
            Self::RuleProgram(error) => write!(formatter, "rule program not accepted: {error}"),
            Self::Graph(error) => error.fmt(formatter),
            Self::Prismatic(error) => error.fmt(formatter),
            Self::Space(error) => error.fmt(formatter),
            Self::Caused { error, cause } => write!(formatter, "{error}: {cause}"),
        }
    }
}

impl std::error::Error for CanonicalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsolvedAssemblySolvePublication(error) => Some(error.as_ref()),
            Self::RuleProgram(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SketchError> for CanonicalError {
    fn from(error: SketchError) -> Self {
        Self::Sketch(error)
    }
}

impl From<PrismaticError> for CanonicalError {
    fn from(error: PrismaticError) -> Self {
        Self::Prismatic(error)
    }
}

impl From<crate::tolerance::InvalidTolerance> for CanonicalError {
    fn from(error: crate::tolerance::InvalidTolerance) -> Self {
        Self::Prismatic(error.into())
    }
}

/// Why a rule program source cannot be bound to or published as a revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleProgramError {
    /// Binding attaches a source to the revision just committed at the head of history.
    NoNewHeadRevision,
    AlreadyBound,
    /// A source-only edit needs a document a program already owns.
    NotProgramOwned,
    EmptySource,
    NonFiniteOverride,
    TooLarge {
        bytes: usize,
    },
}

impl fmt::Display for RuleProgramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoNewHeadRevision => {
                formatter.write_str("no newly committed revision at the head of history")
            }
            Self::AlreadyBound => formatter.write_str("the revision already has a rule program"),
            Self::NotProgramOwned => formatter.write_str("no rule program owns the document"),
            Self::EmptySource => formatter.write_str("the program source is empty"),
            Self::NonFiniteOverride => formatter.write_str("a parameter override is not finite"),
            Self::TooLarge { bytes } => write!(
                formatter,
                "the encoded program is {bytes} bytes, more than {MAX_RULE_PROGRAM_BYTES}"
            ),
        }
    }
}

impl std::error::Error for RuleProgramError {}

impl From<RuleProgramError> for CanonicalError {
    fn from(error: RuleProgramError) -> Self {
        Self::RuleProgram(error)
    }
}

impl From<SpaceError> for CanonicalError {
    fn from(error: SpaceError) -> Self {
        Self::Space(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caused_error_keeps_its_code_and_appends_the_cause() {
        let error = CanonicalError::InvalidSolidToolPlan.because("feature 7 was not found");
        assert_eq!(error.code(), "canonical.invalid_solid_tool_plan");
        assert_eq!(
            error.to_string(),
            format!(
                "{}: feature 7 was not found",
                CanonicalError::InvalidSolidToolPlan
            )
        );
    }
}
