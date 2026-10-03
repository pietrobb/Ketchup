use crate::assembly::{
    ASSEMBLY_MATE_SCHEMA_V1, AssemblyDofDiagnostic, AssemblyDofStatus, AssemblyMate,
    AssemblyMateAttachment, AssemblyMateId, AssemblyMateKind, AssemblyReferenceHealth,
};
use crate::assembly_joint::{
    ASSEMBLY_JOINT_SCHEMA_V1, ASSEMBLY_MOTION_STUDY_SCHEMA_V1, AssemblyJoint, AssemblyJointId,
    AssemblyJointKind, AssemblyJointLimits, AssemblyMotionStudy, AssemblyMotionStudyId,
    joint_motion_states_equal, solve_assembly_joint_kinematics_with_kind_overrides,
    transforms_equivalent,
};
use crate::assembly_recipe::{AssemblyRecipe, AssemblyRecipeError};
use crate::cam::{CamError, CamPlan, CamPlanId};
use crate::drawing::{
    DrawingCircularDimensionKind, DrawingDimensionTolerance, DrawingError,
    DrawingGeometricCharacteristic, DrawingMaterialCondition, DrawingSheet, DrawingSheetId,
    DrawingSource,
};
use crate::exact_brep_graph::{
    ExactBRepGraph, MAX_EXACT_BREP_GRAPH_NODES, MAX_EXACT_BREP_GRAPH_PROFILES,
    MAX_EXACT_BREP_LOFT_CONTROL_POINTS, spatial_sweep_bounds_are_valid, sweep_profile_is_valid,
};
use crate::exact_product::{
    BodySubshapeRef, ExactProducerCompilation, ExactProducerEvidenceContext,
    ExactReferenceResolution, ExactResultRegistry, accepts_planar_offset_solved_region,
    accepts_sweep_segment_profile, canonical_reference_lineage_digest, exact_planar_offset_profile,
};
pub use crate::graph::{
    CanonicalOverride, DerivedIdentity, DerivedOutput, EvaluationIdentity, EvaluationReport,
    EvaluationStatus, EvaluatorNode, EvaluatorNodeKind, GraphError, OverrideMergePolicy,
    OverrideParameterSpec, PortSpec, RuleOutput, SlotPath, SlotResolution, SlotSegment, ValueType,
};
use crate::graph::{
    ExpressionAst, evaluate_affected, evaluate_graph, resolve_derived_identity,
    validate_graph as validate_typed_graph,
};
use crate::import::{
    ImportContractError, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportUnitAuthority,
};
use crate::mechanical_contract::{
    MECHANICAL_CONDITION_SCHEMA_V1, MECHANICAL_INTERFACE_SCHEMA_V1, MechanicalCondition,
    MechanicalConditionId, MechanicalConditionKind, MechanicalInterface, MechanicalInterfaceId,
};
use crate::mechanical_coupling::{
    ASSEMBLY_MOTION_COUPLING_SCHEMA_V1, AssemblyMotionCoupling, AssemblyMotionCouplingId,
    CoupledJointKind,
};
use crate::pin_joint::{PinJointContract, PinJointError, PinJointId, project_pin_joint_contract};
use crate::sheet_metal::{SheetMetalError, SheetMetalSpec};
use crate::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceCoordinateFrame, ClearanceOwner,
    ClearanceSeverity, ClearanceVolumeId, SpaceError, SpaceId,
};
use crate::tolerance::{APPROXIMATION, MAX_COORDINATE_MM, ROUNDING, TolerancePolicy};
use crate::topology::{TopologicalElementKind, TopologicalElementRef};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use ketchup_geometry::prismatic::{CanonicalJoint, JointId, PrismaticError};
use ketchup_geometry::sketch::{
    CutStart, FeatureDirection, FeatureExtent, FeatureExtentEnd, PadOperation, PadProfile, PadSpec,
    PrincipalPlane, SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity,
    SketchError, SketchOffsetSide, SketchPointKind, SketchSpec, SolvedSketchRegionProfile,
    WorkplaneFrame, WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub use ketchup_geometry::dimension::Dimension;
use ketchup_geometry::dimension::DimensionError;

pub const COMMAND_SCHEMA_V1: &str = "ketchup.command.v1";
pub const TOLERANCE_PROFILE_V1: &str = "ketchup.tolerance.r0-v1";
pub const MAX_SOLID_TOOL_RESULT_FEATURES: usize =
    2 * (MAX_EXACT_BREP_GRAPH_NODES + 2 * MAX_EXACT_BREP_GRAPH_PROFILES) + 3;

pub use ketchup_geometry::id::{DefinitionId, DocumentId, FeatureId, NodeId};
use ketchup_geometry::typed_id;

typed_id!(BodyId);
typed_id!(OccurrenceId);
typed_id!(GroupId);
typed_id!(TagId);
typed_id!(ClassificationDimensionId);
typed_id!(ClassificationCategoryId);
typed_id!(CollectionId);
typed_id!(LocalOccurrenceId);
typed_id!(LocalGroupId);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct LocalOccurrenceKey {
    pub definition_id: DefinitionId,
    pub local_id: LocalOccurrenceId,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct LocalGroupKey {
    pub definition_id: DefinitionId,
    pub local_id: LocalGroupId,
}

mod assembly_validation;
mod command;
mod contact_joint;
pub use contact_joint::ContactJoint;
pub(crate) mod digest_v3;
mod entities;
mod error;
mod feature_validation;
mod group_conversion;
mod instance_path;
mod parameters;
mod persistent_dimension;
mod planar_face;
mod product_validation;
mod proposal;
mod proposal_analysis;
mod revision;
mod scene_query;
mod snapshot;
mod solid_tool;
mod stable_digest;
mod store;
mod support;
pub(crate) use ketchup_geometry::derived::{derived, identity_form};
pub(crate) use stable_digest::digest_product;

pub(crate) use assembly_validation::*;
pub use command::*;
pub use entities::*;
pub use error::*;
pub use feature_validation::*;
use group_conversion::*;
pub use instance_path::{InstancePath, InstancePathStep};
use parameters::*;
pub use persistent_dimension::*;
use planar_face::*;
use product_validation::*;
pub use proposal::*;
pub(crate) use proposal_analysis::*;
pub use revision::*;
pub use scene_query::*;
pub use snapshot::*;
use solid_tool::*;
use stable_digest::{StableDigest, digest_feature, digest_snapshot};
pub use store::*;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProductModel {
    pub(crate) document_id: DocumentId,
    pub(crate) units: UnitSystem,
    #[serde(default, skip_serializing_if = "TolerancePolicy::is_default")]
    pub(crate) tolerance: TolerancePolicy,
    #[serde(flatten)]
    pub(crate) support: support::SupportDeclarations,
    pub(crate) evaluator_nodes: BTreeMap<NodeId, Arc<EvaluatorNode>>,
    pub(crate) overrides: BTreeMap<u64, Arc<CanonicalOverride>>,
    pub(crate) feature_parameter_bindings:
        BTreeMap<FeatureParameterTarget, Arc<FeatureParameterBinding>>,
    pub(crate) feature_parameter_provenance:
        BTreeMap<FeatureParameterTarget, Arc<FeatureParameterProvenance>>,
    pub(crate) joints: BTreeMap<JointId, Arc<CanonicalJoint>>,
    pub(crate) spaces: BTreeMap<SpaceId, Arc<CanonicalSpace>>,
    pub(crate) clearance_volumes: BTreeMap<ClearanceVolumeId, Arc<CanonicalClearanceVolume>>,
    pub(crate) cam_plans: BTreeMap<CamPlanId, Arc<CamPlan>>,
    pub(crate) pin_joints: BTreeMap<PinJointId, Arc<PinJointContract>>,
    pub(crate) assembly_recipe: Option<Arc<AssemblyRecipe>>,
    #[serde(serialize_with = "derived")]
    pub(crate) exact_reference_evidence: BTreeMap<String, Arc<BodySubshapeRef>>,
    pub(crate) persistent_dimensions: BTreeMap<PersistentDimensionId, Arc<PersistentDimension>>,
    pub(crate) tags: BTreeMap<TagId, Arc<Tag>>,
    pub(crate) classification_dimensions:
        BTreeMap<ClassificationDimensionId, Arc<ClassificationDimension>>,
    pub(crate) classification_assignments:
        BTreeMap<(OccurrenceId, ClassificationDimensionId), ClassificationCategoryId>,
    pub(crate) collections: BTreeMap<CollectionId, Arc<Collection>>,
    pub(crate) import_receipts: BTreeMap<ImportId, Arc<ImportReceipt>>,
    pub(crate) definitions: BTreeMap<DefinitionId, Arc<Definition>>,
    pub(crate) features: BTreeMap<FeatureId, Arc<Feature>>,
    pub(crate) body_feature_suppression: BTreeMap<(DefinitionId, BodyId), BTreeSet<FeatureId>>,
    pub(crate) occurrences: BTreeMap<OccurrenceId, Arc<Occurrence>>,
    pub(crate) grounded_occurrences: BTreeSet<OccurrenceId>,
    pub(crate) assembly_mates: BTreeMap<AssemblyMateId, Arc<AssemblyMate>>,
    pub(crate) assembly_joints: BTreeMap<AssemblyJointId, Arc<AssemblyJoint>>,
    pub(crate) assembly_motion_couplings:
        BTreeMap<AssemblyMotionCouplingId, Arc<AssemblyMotionCoupling>>,
    pub(crate) assembly_motion_studies: BTreeMap<AssemblyMotionStudyId, Arc<AssemblyMotionStudy>>,
    pub(crate) mechanical_interfaces: BTreeMap<MechanicalInterfaceId, Arc<MechanicalInterface>>,
    pub(crate) mechanical_conditions: BTreeMap<MechanicalConditionId, Arc<MechanicalCondition>>,
    pub(crate) drawing_sheets: BTreeMap<DrawingSheetId, Arc<DrawingSheet>>,
    pub(crate) groups: BTreeMap<GroupId, Arc<Group>>,
    pub(crate) local_occurrences: BTreeMap<LocalOccurrenceKey, Arc<LocalOccurrence>>,
    pub(crate) local_groups: BTreeMap<LocalGroupKey, Arc<LocalGroup>>,
    pub(crate) production_codes: BTreeMap<InstancePath, String>,
    pub(crate) instance_transform_overrides: BTreeMap<InstancePath, Transform>,
    #[serde(skip)]
    pub(crate) canonical_digest: DigestCache,
    #[serde(skip)]
    pub(crate) exact_graphs: ExactGraphCache,
}

/// Exact B-Rep graphs compiled from one immutable product model.
/// Cached per immutable revision; clones start empty, as with [`DigestCache`].
#[derive(Default)]
pub(crate) struct ExactGraphCache(std::sync::Mutex<ExactGraphsByProducer>);
type ExactGraphsByProducer = BTreeMap<(u64, DefinitionId, FeatureId), Option<Arc<ExactBRepGraph>>>;

impl Clone for ExactGraphCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// The canonical digest of one immutable product model, computed at most once.
///
/// Immutable snapshots memoize the O(document) hash; clones for new revisions
/// start with an empty cache.
#[derive(Default)]
pub(crate) struct DigestCache(OnceLock<String>);

impl Clone for DigestCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Default for ProductModel {
    fn default() -> Self {
        Self {
            document_id: allocate_document_id(),
            units: UnitSystem::Millimetres,
            tolerance: TolerancePolicy::default(),
            support: support::SupportDeclarations::default(),
            evaluator_nodes: BTreeMap::new(),
            overrides: BTreeMap::new(),
            feature_parameter_bindings: BTreeMap::new(),
            feature_parameter_provenance: BTreeMap::new(),
            joints: BTreeMap::new(),
            spaces: BTreeMap::new(),
            clearance_volumes: BTreeMap::new(),
            cam_plans: BTreeMap::new(),
            pin_joints: BTreeMap::new(),
            assembly_recipe: None,
            exact_reference_evidence: BTreeMap::new(),
            persistent_dimensions: BTreeMap::new(),
            tags: BTreeMap::new(),
            classification_dimensions: BTreeMap::new(),
            classification_assignments: BTreeMap::new(),
            collections: BTreeMap::new(),
            import_receipts: BTreeMap::new(),
            definitions: BTreeMap::new(),
            features: BTreeMap::new(),
            body_feature_suppression: BTreeMap::new(),
            occurrences: BTreeMap::new(),
            grounded_occurrences: BTreeSet::new(),
            assembly_mates: BTreeMap::new(),
            assembly_joints: BTreeMap::new(),
            assembly_motion_couplings: BTreeMap::new(),
            assembly_motion_studies: BTreeMap::new(),
            mechanical_interfaces: BTreeMap::new(),
            mechanical_conditions: BTreeMap::new(),
            drawing_sheets: BTreeMap::new(),
            groups: BTreeMap::new(),
            local_occurrences: BTreeMap::new(),
            local_groups: BTreeMap::new(),
            production_codes: BTreeMap::new(),
            instance_transform_overrides: BTreeMap::new(),
            canonical_digest: DigestCache::default(),
            exact_graphs: ExactGraphCache::default(),
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

#[cfg(test)]
mod parameter_contract_tests {
    use super::*;
    use ketchup_geometry::sketch::{
        FeatureDirection, SketchConstraint, SketchEntityId, SketchPointRef, SketchRegionId,
    };

    fn dimension(value: f64) -> Dimension {
        Dimension::new(value.to_string(), value).unwrap()
    }

    fn assert_descriptors_have_read_write_accessors(kind: FeatureKind, tolerance_mm: f64) {
        validate_feature_kind(&kind, tolerance_mm).unwrap();
        let descriptors = kind.parameter_descriptors();
        assert!(!descriptors.is_empty());
        for descriptor in descriptors {
            let value = feature_kind_parameter_value(&kind, descriptor.path().as_str())
                .unwrap_or_else(|| panic!("missing reader for {}", descriptor.path().as_str()));
            let target = FeatureParameterTarget::new(
                FeatureId(99),
                descriptor.path().as_str(),
                descriptor.value_type(),
            )
            .unwrap();
            let mut updated = kind.clone();
            assert!(
                set_feature_kind_parameter(&mut updated, &target, &dimension(value), tolerance_mm)
                    .unwrap(),
                "missing writer for {}",
                descriptor.path().as_str()
            );
            validate_feature_kind(&updated, tolerance_mm).unwrap_or_else(|error| {
                panic!(
                    "writer for {} produced invalid feature: {error}",
                    descriptor.path().as_str()
                )
            });
            assert_eq!(
                feature_kind_parameter_value(&updated, descriptor.path().as_str()),
                Some(value)
            );
        }
    }

    #[test]
    fn every_structural_descriptor_has_a_valid_read_write_round_trip() {
        let tolerance_mm = crate::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
        let sketch = FeatureKind::Sketch(SketchSpec {
            workplane: FeatureId(1),
            entities: vec![SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: [3.0, 4.0],
                radius_mm: 2.0,
            }],
            constraints: vec![
                SketchConstraint {
                    id: SketchConstraintId(1),
                    kind: SketchConstraintKind::Radius {
                        entity: SketchEntityId(1),
                        value: dimension(2.0),
                    },
                },
                SketchConstraint {
                    id: SketchConstraintId(2),
                    kind: SketchConstraintKind::FixedPoint {
                        point: SketchPointRef {
                            entity: SketchEntityId(1),
                            point: SketchPointKind::Center,
                        },
                        position_mm: [3.0, 4.0],
                    },
                },
            ],
        });
        let kinds = vec![
            FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::Offset {
                    base: FeatureId(1),
                    distance: dimension(8.0),
                },
                frame: WorkplaneFrame::principal(PrincipalPlane::Xy),
            }),
            sketch,
            FeatureKind::polygon(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]]),
            FeatureKind::Profile {
                segments: vec![ProfileSegment::Line {
                    start_mm: [0.0, 0.0],
                    end_mm: [4.0, 2.0],
                }],
                closed: false,
            },
            FeatureKind::closed_spline(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]]),
            FeatureKind::extrusion(FeatureId(1), dimension(12.0)),
            FeatureKind::Pad(PadSpec {
                profile: PadProfile::SketchRegion {
                    sketch: FeatureId(1),
                    region: SketchRegionId(1),
                },
                direction: FeatureDirection::AlongNormal,
                extent: FeatureExtent::Bidirectional {
                    along: FeatureExtentEnd::Blind(dimension(6.0)),
                    opposite: FeatureExtentEnd::Blind(dimension(3.0)),
                },
                operation: PadOperation::NewBody,
            }),
            FeatureKind::Revolve {
                profile: FeatureId(1),
                axis_start_mm: [0.0, 0.0],
                axis_end_mm: [0.0, 1.0],
                angle_degrees: 180.0,
            },
            FeatureKind::pocket(FeatureId(2), FeatureId(1), dimension(5.0)),
            FeatureKind::PlanarOffset {
                profile: FeatureId(1),
                distance: dimension(2.0),
            },
            FeatureKind::Loft {
                sections: vec![
                    LoftSection {
                        profile: FeatureId(1),
                        elevation_mm: 0.0,
                    },
                    LoftSection {
                        profile: FeatureId(2),
                        elevation_mm: 10.0,
                    },
                ],
                guide: None,
                continuity: LoftContinuity::Position,
            },
        ];
        for kind in kinds {
            assert_descriptors_have_read_write_accessors(kind, tolerance_mm);
        }
    }
}
