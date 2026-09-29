//! Frozen reader for native documents written before the serde snapshot codec.
//!
//! Schemas 0..=97 stored every type field by field. This module keeps that reader
//! unchanged so existing files still open; the loader migrates its output to the
//! current format. It has no writer and must not grow new schema branches.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::assembly::{
    ASSEMBLY_MATE_SCHEMA_V1, AssemblyMate, AssemblyMateAttachment, AssemblyMateEndpoint,
    AssemblyMateId, AssemblyMateKind, AssemblyReferenceHealth, AxialAttachment,
    AxialAttachmentKind, PlanarFaceAttachment,
};
use crate::assembly_joint::{
    ASSEMBLY_JOINT_SCHEMA_V1, ASSEMBLY_MOTION_STUDY_SCHEMA_V1, AssemblyJoint, AssemblyJointAxis,
    AssemblyJointId, AssemblyJointKind, AssemblyJointLimits, AssemblyMotionDriver,
    AssemblyMotionStudy, AssemblyMotionStudyId,
};
use crate::assembly_recipe::{
    AssemblyRecipe, RecipeEditScope, RecipeFaceRef, RecipeJoinery, RecipeKey, RecipeOwnedFeature,
    RecipeParameter, RecipeParameterUnit, RecipePart, RecipePartMobility, RecipeRelation,
    RecipeRelationKind, RecognizedRecipeFeatureKind,
};
use crate::cam::{
    CamCutParameters, CamPlan, CamPlanId, CamSetup, CamStock, CamTarget, CamTool, CamToolKind,
    CamUnits, CamWorkOffset,
};
use crate::document::{
    Body, BodyId, BodyKind, BooleanOperation, CanonicalError, ChamferEdgeSide, ChamferMode,
    ClassificationCategory, ClassificationCategoryId, ClassificationDimension,
    ClassificationDimensionId, Collection, CollectionId, Definition, DefinitionId, Dimension,
    DimensionDisplayUnit, DimensionPresentation, EdgeFinishKind, EdgeRef, EvaluationIdentity,
    EvaluatorNode, ExactReferenceConversionConsequence, ExactToMeshConversion, FaceRef, Feature,
    FeatureBodyOwnership, FeatureId, FeatureKind, FeatureParameterBinding,
    FeatureParameterProvenance, FeatureParameterTarget, FilletRadiusStation, Group, GroupId,
    ImportedExactBodySpec, InstancePath, InstancePathStep, LocalGroup, LocalGroupId, LocalGroupKey,
    LocalOccurrence, LocalOccurrenceId, LocalOccurrenceKey, LoftContinuity, LoftSection,
    MeshAuthority, MeshBodySpec, NodeId, Occurrence, OccurrenceId, ParameterPath,
    ParameterValueType, PersistentDimension, PersistentDimensionId, PersistentDimensionTarget,
    ProductModel, ProfileEdgeReference, ProfileFaceReference, ProfileSegment, ShellDirection,
    SpatialPathSegment, SurfaceBodySpec, Tag, TagId, Transform, UnitSystem, WeldmentJointPolicy,
    WeldmentJointPrimary, WeldmentJointSpec, WeldmentMemberSpec,
};
use crate::drawing::{
    DrawingAngularDimension, DrawingAnnotations, DrawingBomBalloon, DrawingBomBalloonId,
    DrawingCircularDimension, DrawingCircularDimensionKind, DrawingDatumId, DrawingDatumReference,
    DrawingDatumSymbol, DrawingDetailRegion, DrawingDimensionId, DrawingDimensionTolerance,
    DrawingFeatureControlFrame, DrawingFeatureControlFrameId, DrawingGeometricCharacteristic,
    DrawingLinearDimension, DrawingMargins, DrawingMaterialCondition, DrawingNote, DrawingNoteId,
    DrawingPageOrientation, DrawingPageSize, DrawingPageTemplate, DrawingScale,
    DrawingSectionPlane, DrawingSheet, DrawingSheetId, DrawingSource, DrawingTitleBlock,
    DrawingViewFrame, MAX_DRAWING_BOM_BALLOONS, MAX_DRAWING_DIMENSIONS, MAX_DRAWING_NOTES,
    MAX_DRAWING_VIEWS, ORTHOGRAPHIC_DRAWING_SCHEMA_V1, ORTHOGRAPHIC_DRAWING_SCHEMA_V2,
    OrthographicViewKind,
};
use crate::exact_product::{BODY_SUBSHAPE_REF_SCHEMA_V1, BodySubshapeRef, ReferenceStability};
use crate::graph::{
    CanonicalOverride, DerivedIdentity, OverrideParameterSpec, PortSpec, RuleOutput, SlotPath,
    SlotResolution, SlotSegment,
};
use crate::import::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportId, ImportLengthUnit,
    ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
    MAX_IMPORT_DIAGNOSTICS, MAX_IMPORT_OUTPUTS,
};
use crate::joinery::{
    DowelJointContract, DowelJointFace, DowelJointId, DowelPhysicalHolePair, DowelSpec,
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
use crate::sheet_metal::{SheetMetalEdge, SheetMetalFlange, SheetMetalSpec};
use crate::sketch::{
    CutStart, FeatureDirection, FeatureExtent, FeatureExtentEnd, MAX_SKETCH_CONSTRAINTS,
    MAX_SKETCH_ENTITIES, PadOperation, PadProfile, PadSpec, PrincipalPlane, SketchConstraint,
    SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind,
    SketchPointRef, SketchRegionId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use crate::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceOwner, ClearanceSeverity, ClearanceVolumeId,
    SpaceId,
};
use crate::topology::TopologicalElementRef;

use super::{MAGIC, MigrationLoss, PersistenceError};

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
const PLANAR_FACE_ATTACHMENT_SCHEMA: u16 = 50;
const AXIAL_ATTACHMENT_SCHEMA: u16 = 51;
const FREE_WORKPLANE_SCHEMA: u16 = 52;
const OCCURRENCE_COLOR_SCHEMA: u16 = 53;
const GLB_IMPORT_SCHEMA: u16 = 54;
const DRAWING_PAGE_CONTRACT_SCHEMA: u16 = 55;
const DRAWING_VIEW_CONTRACT_SCHEMA: u16 = 56;
const DRAWING_SECTION_CONTRACT_SCHEMA: u16 = 57;
const DRAWING_DETAIL_CONTRACT_SCHEMA: u16 = 58;
const DRAWING_DIMENSION_CONTRACT_SCHEMA: u16 = 59;
const DRAWING_TOLERANCE_CONTRACT_SCHEMA: u16 = 60;
const DRAWING_ANNOTATION_CONTRACT_SCHEMA: u16 = 61;
const SKETCH_PROJECTION_SCHEMA: u16 = 62;
const SKETCH_CONSTRUCTION_SCHEMA: u16 = 63;
const HELICAL_ASSEMBLY_JOINT_SCHEMA: u16 = 64;
const IGES_IMPORT_SCHEMA: u16 = 65;
const CONSTRUCTION_GEOMETRY_SCHEMA: u16 = 66;
const CONSTRUCTION_AXIS_SCHEMA: u16 = 67;
const CONSTRUCTION_PLANE_SCHEMA: u16 = 68;
const CONSTRUCTION_PLANE_WORKPLANE_SCHEMA: u16 = 69;
const LOFT_GUIDE_CONTINUITY_SCHEMA: u16 = 70;
const SHELL_DIRECTION_SCHEMA: u16 = 71;
const VARIABLE_FILLET_SCHEMA: u16 = 72;
const ADVANCED_CHAMFER_SCHEMA: u16 = 73;
const NESTED_ASSEMBLY_PATH_SCHEMA: u16 = 74;
const NESTED_DRAWING_PATH_SCHEMA: u16 = 75;
const NESTED_INSTANCE_TRANSFORM_SCHEMA: u16 = 76;
const TYPED_DRAWING_DIMENSION_SCHEMA: u16 = 77;
const DRAWING_GDT_SCHEMA: u16 = 78;
const DRAWING_BOM_BALLOON_SCHEMA: u16 = 79;
const IMPORTED_EXACT_PART_SCHEMA: u16 = 80;
const SHEET_METAL_SCHEMA: u16 = 81;
const WELDMENT_MEMBER_SCHEMA: u16 = 82;
const WELDMENT_JOINT_SCHEMA: u16 = 83;
const SURFACE_BODY_SCHEMA: u16 = 84;
const SURFACE_OPERATION_SCHEMA: u16 = 85;
const SURFACE_KNIT_SCHEMA: u16 = 86;
const SURFACE_THICKEN_SCHEMA: u16 = 87;
const IMPORTED_EXACT_BODY_KIND_SCHEMA: u16 = 88;
const CAM_PLAN_SCHEMA: u16 = 89;
const DOWEL_JOINERY_SCHEMA: u16 = 90;
const PRODUCTION_CODE_SCHEMA: u16 = 91;
const DOWEL_PHYSICAL_HOLE_BINDING_SCHEMA: u16 = 92;
const ASSEMBLY_RECIPE_SCHEMA: u16 = 93;
const DOWEL_PAIR_OFFSET_SCHEMA: u16 = 94;
const PROFILE_EDGE_REFERENCE_SCHEMA: u16 = 95;
const PROFILE_FACE_REFERENCE_SCHEMA: u16 = 96;
const PROFILE_SHELL_FACE_SCHEMA: u16 = 97;
/// The last schema written by the field-by-field binary writer.
const LAST_LEGACY_SCHEMA: u16 = PROFILE_SHELL_FACE_SCHEMA;
const COLLECTION_SCHEMA: u16 = 15;
const TAG_SCHEMA: u16 = 14;
const PERSISTENT_DIMENSION_SCHEMA: u16 = 13;
const PROFILE_CONSTRAINT_SCHEMA: u16 = 12;
const PARAMETRIC_PROVENANCE_SCHEMA: u16 = 11;
const PARAMETRIC_BINDING_SCHEMA: u16 = 10;
const BOOLEAN_SCHEMA: u16 = 9;
const EDGE_FINISH_SCHEMA: u16 = 8;
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
    boolean_intersect: bool,
    boolean_split: bool,
    planar_offset: bool,
    sweep: bool,
    loft_spline: bool,
    import_receipts: bool,
    imported_exact_body: bool,
    imported_exact_part: bool,
    imported_topology_counts: bool,
    imported_exact_body_kind: bool,
    topological_feature_references: bool,
    general_parameter_paths: bool,
    sketchup_scene: bool,
    workplane_sketch: bool,
    assembly_contract: bool,
    orthographic_drawing: bool,
    drawing_page_contract: bool,
    drawing_view_contract: bool,
    drawing_section_contract: bool,
    drawing_detail_contract: bool,
    drawing_dimension_contract: bool,
    drawing_tolerance_contract: bool,
    drawing_annotation_contract: bool,
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
    planar_face_attachments: bool,
    axial_attachments: bool,
    free_workplanes: bool,
    occurrence_colors: bool,
    glb_import: bool,
    iges_import: bool,
    construction_geometry: bool,
    construction_axis: bool,
    construction_plane: bool,
    construction_plane_workplanes: bool,
    loft_guide_continuity: bool,
    shell_direction: bool,
    variable_fillet: bool,
    advanced_chamfer: bool,
    nested_assembly_paths: bool,
    nested_drawing_paths: bool,
    nested_instance_transforms: bool,
    typed_drawing_dimensions: bool,
    drawing_gdt: bool,
    drawing_bom_balloons: bool,
    sketch_projection: bool,
    sketch_construction: bool,
    helical_assembly_joints: bool,
    sheet_metal: bool,
    weldment_member: bool,
    weldment_joint: bool,
    surface_body: bool,
    surface_operations: bool,
    surface_knit: bool,
    surface_thicken: bool,
    cam_plans: bool,
    dowel_joinery: bool,
    production_codes: bool,
    dowel_physical_hole_bindings: bool,
    dowel_pair_offsets: bool,
    profile_edge_references: bool,
    profile_face_references: bool,
    profile_shell_faces: bool,
    assembly_recipe: bool,
}

impl ProductSchemaCapabilities {
    const PRODUCT_V2: Self = Self {
        current: false,
        exact_evidence: false,
        through_cut: false,
        revolve: false,
        shell: false,
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
        boolean_intersect: false,
        boolean_split: false,
        planar_offset: false,
        sweep: false,
        loft_spline: false,
        import_receipts: false,
        imported_exact_body: false,
        imported_exact_part: false,
        imported_topology_counts: false,
        imported_exact_body_kind: false,
        topological_feature_references: false,
        general_parameter_paths: false,
        sketchup_scene: false,
        workplane_sketch: false,
        assembly_contract: false,
        orthographic_drawing: false,
        drawing_page_contract: false,
        drawing_view_contract: false,
        drawing_section_contract: false,
        drawing_detail_contract: false,
        drawing_dimension_contract: false,
        drawing_tolerance_contract: false,
        drawing_annotation_contract: false,
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
        planar_face_attachments: false,
        axial_attachments: false,
        free_workplanes: false,
        occurrence_colors: false,
        glb_import: false,
        iges_import: false,
        construction_geometry: false,
        construction_axis: false,
        construction_plane: false,
        construction_plane_workplanes: false,
        loft_guide_continuity: false,
        shell_direction: false,
        variable_fillet: false,
        advanced_chamfer: false,
        nested_assembly_paths: false,
        nested_drawing_paths: false,
        nested_instance_transforms: false,
        typed_drawing_dimensions: false,
        drawing_gdt: false,
        drawing_bom_balloons: false,
        sketch_projection: false,
        sketch_construction: false,
        helical_assembly_joints: false,
        sheet_metal: false,
        weldment_member: false,
        weldment_joint: false,
        surface_body: false,
        surface_operations: false,
        surface_knit: false,
        surface_thicken: false,
        cam_plans: false,
        dowel_joinery: false,
        production_codes: false,
        dowel_physical_hole_bindings: false,
        dowel_pair_offsets: false,
        profile_edge_references: false,
        profile_face_references: false,
        profile_shell_faces: false,
        assembly_recipe: false,
    };

    const fn current(schema: u16) -> Self {
        Self {
            current: schema >= THROUGH_CUT_SCHEMA,
            exact_evidence: schema >= EXACT_EVIDENCE_SCHEMA,
            through_cut: schema >= THROUGH_CUT_SCHEMA,
            revolve: schema >= REVOLVE_SCHEMA,
            shell: schema >= SHELL_SCHEMA,
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
            boolean_intersect: schema >= BOOLEAN_INTERSECT_SCHEMA,
            boolean_split: schema >= BOOLEAN_SPLIT_SCHEMA,
            planar_offset: schema >= PLANAR_OFFSET_SCHEMA,
            sweep: schema >= SWEEP_SCHEMA,
            loft_spline: schema >= LOFT_SPLINE_SCHEMA,
            import_receipts: schema >= IMPORT_RECEIPT_SCHEMA,
            imported_exact_body: schema >= IMPORTED_EXACT_BODY_SCHEMA,
            imported_exact_part: schema >= IMPORTED_EXACT_PART_SCHEMA,
            imported_topology_counts: schema >= IMPORTED_TOPOLOGY_COUNTS_SCHEMA,
            imported_exact_body_kind: schema >= IMPORTED_EXACT_BODY_KIND_SCHEMA,
            topological_feature_references: schema >= TOPOLOGICAL_FEATURE_REFERENCE_SCHEMA,
            general_parameter_paths: schema >= GENERAL_PARAMETER_PATH_SCHEMA,
            sketchup_scene: schema >= SKETCHUP_SCENE_SCHEMA,
            workplane_sketch: schema >= WORKPLANE_SKETCH_SCHEMA,
            assembly_contract: schema >= ASSEMBLY_CONTRACT_SCHEMA,
            orthographic_drawing: schema >= ORTHOGRAPHIC_DRAWING_SCHEMA,
            drawing_page_contract: schema >= DRAWING_PAGE_CONTRACT_SCHEMA,
            drawing_view_contract: schema >= DRAWING_VIEW_CONTRACT_SCHEMA,
            drawing_section_contract: schema >= DRAWING_SECTION_CONTRACT_SCHEMA,
            drawing_detail_contract: schema >= DRAWING_DETAIL_CONTRACT_SCHEMA,
            drawing_dimension_contract: schema >= DRAWING_DIMENSION_CONTRACT_SCHEMA,
            drawing_tolerance_contract: schema >= DRAWING_TOLERANCE_CONTRACT_SCHEMA,
            drawing_annotation_contract: schema >= DRAWING_ANNOTATION_CONTRACT_SCHEMA,
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
            planar_face_attachments: schema >= PLANAR_FACE_ATTACHMENT_SCHEMA,
            axial_attachments: schema >= AXIAL_ATTACHMENT_SCHEMA,
            free_workplanes: schema >= FREE_WORKPLANE_SCHEMA,
            occurrence_colors: schema >= OCCURRENCE_COLOR_SCHEMA,
            glb_import: schema >= GLB_IMPORT_SCHEMA,
            iges_import: schema >= IGES_IMPORT_SCHEMA,
            construction_geometry: schema >= CONSTRUCTION_GEOMETRY_SCHEMA,
            construction_axis: schema >= CONSTRUCTION_AXIS_SCHEMA,
            construction_plane: schema >= CONSTRUCTION_PLANE_SCHEMA,
            construction_plane_workplanes: schema >= CONSTRUCTION_PLANE_WORKPLANE_SCHEMA,
            loft_guide_continuity: schema >= LOFT_GUIDE_CONTINUITY_SCHEMA,
            shell_direction: schema >= SHELL_DIRECTION_SCHEMA,
            variable_fillet: schema >= VARIABLE_FILLET_SCHEMA,
            advanced_chamfer: schema >= ADVANCED_CHAMFER_SCHEMA,
            nested_assembly_paths: schema >= NESTED_ASSEMBLY_PATH_SCHEMA,
            nested_drawing_paths: schema >= NESTED_DRAWING_PATH_SCHEMA,
            nested_instance_transforms: schema >= NESTED_INSTANCE_TRANSFORM_SCHEMA,
            typed_drawing_dimensions: schema >= TYPED_DRAWING_DIMENSION_SCHEMA,
            drawing_gdt: schema >= DRAWING_GDT_SCHEMA,
            drawing_bom_balloons: schema >= DRAWING_BOM_BALLOON_SCHEMA,
            sketch_projection: schema >= SKETCH_PROJECTION_SCHEMA,
            sketch_construction: schema >= SKETCH_CONSTRUCTION_SCHEMA,
            helical_assembly_joints: schema >= HELICAL_ASSEMBLY_JOINT_SCHEMA,
            sheet_metal: schema >= SHEET_METAL_SCHEMA,
            weldment_member: schema >= WELDMENT_MEMBER_SCHEMA,
            weldment_joint: schema >= WELDMENT_JOINT_SCHEMA,
            surface_body: schema >= SURFACE_BODY_SCHEMA,
            surface_operations: schema >= SURFACE_OPERATION_SCHEMA,
            surface_knit: schema >= SURFACE_KNIT_SCHEMA,
            surface_thicken: schema >= SURFACE_THICKEN_SCHEMA,
            cam_plans: schema >= CAM_PLAN_SCHEMA,
            dowel_joinery: schema >= DOWEL_JOINERY_SCHEMA,
            production_codes: schema >= PRODUCTION_CODE_SCHEMA,
            dowel_physical_hole_bindings: schema >= DOWEL_PHYSICAL_HOLE_BINDING_SCHEMA,
            dowel_pair_offsets: schema >= DOWEL_PAIR_OFFSET_SCHEMA,
            profile_edge_references: schema >= PROFILE_EDGE_REFERENCE_SCHEMA,
            profile_face_references: schema >= PROFILE_FACE_REFERENCE_SCHEMA,
            profile_shell_faces: schema >= PROFILE_SHELL_FACE_SCHEMA,
            assembly_recipe: schema >= ASSEMBLY_RECIPE_SCHEMA,
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

pub(super) struct Decoded {
    /// Features stored as a profile of corner points (read as closed chains of lines).
    pub(super) point_profiles: BTreeSet<FeatureId>,
    pub(super) revision_id: u64,
    pub(super) product: ProductModel,
    pub(super) migration_losses: Vec<MigrationLoss>,
}

/// Completes what old records stored differently from the current model, which needs
/// the whole product: an old pocket of a sketch was cut from the sketch plane, not from
/// the target face; old extrusion and pocket records named their distance `height` and
/// `depth` where a pad names it `extent.distance`; old point profiles named a corner
/// `points.N` where a profile names it `segments.N.start`; old spline profiles named
/// their points `control_points.N` where the one closed spline segment names them
/// `segments.0.start` and `segments.0.points.N`. The names are renamed in every parameter
/// target that points at such a feature.
pub(super) fn complete_old_records(
    product: &ProductModel,
    point_profiles: &BTreeSet<FeatureId>,
) -> Result<ProductModel, ciborium::value::Error> {
    let mut product = product.clone();
    let sketches = product
        .features
        .values()
        .filter(|feature| matches!(feature.kind, FeatureKind::Sketch(_)))
        .map(|feature| feature.id)
        .collect::<BTreeSet<_>>();
    for feature in product.features.values_mut() {
        let pocket_of_sketch = matches!(
            &feature.kind,
            FeatureKind::Pad(PadSpec {
                profile: PadProfile::Feature(profile),
                operation: PadOperation::Cut { start: CutStart::TargetFace, .. },
                ..
            }) if sketches.contains(profile)
        );
        if pocket_of_sketch
            && let FeatureKind::Pad(PadSpec {
                operation: PadOperation::Cut { start, .. },
                ..
            }) = &mut Arc::make_mut(feature).kind
        {
            *start = CutStart::ProfilePlane;
        }
    }
    let blind_pads = product
        .features
        .values()
        .filter(|feature| {
            matches!(
                &feature.kind,
                FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(_),
                    extent: FeatureExtent::Blind(_),
                    ..
                })
            )
        })
        .map(|feature| feature.id)
        .collect::<BTreeSet<_>>();
    // Before segment profiles could hold splines, a closed spline was its own record.
    let spline_profiles = product
        .features
        .values()
        .filter(|feature| feature.kind.closed_spline_points().is_some())
        .map(|feature| feature.id)
        .collect::<BTreeSet<_>>();
    if blind_pads.is_empty() && point_profiles.is_empty() && spline_profiles.is_empty() {
        return Ok(product);
    }
    rename_parameter_paths(&product, |feature, path| {
        if blind_pads.contains(&feature) && matches!(path, "height" | "depth") {
            return Some("extent.distance".to_owned());
        }
        if spline_profiles.contains(&feature) {
            let (index, axis) = path.strip_prefix("control_points.")?.split_once('.')?;
            return Some(match index {
                "0" => format!("segments.0.start.{axis}"),
                _ => format!("segments.0.points.{index}.{axis}"),
            });
        }
        let corner = path.strip_prefix("points.")?;
        point_profiles
            .contains(&feature)
            .then(|| format!("segments.{}", corner.replacen('.', ".start.", 1)))
    })
}

fn rename_parameter_paths(
    product: &ProductModel,
    rename: impl Fn(FeatureId, &str) -> Option<String>,
) -> Result<ProductModel, ciborium::value::Error> {
    fn visit(value: &mut ciborium::Value, rename: &dyn Fn(FeatureId, &str) -> Option<String>) {
        match value {
            ciborium::Value::Map(entries) => {
                let field = |entries: &[(ciborium::Value, ciborium::Value)], name: &str| {
                    entries
                        .iter()
                        .position(|(key, _)| key.as_text() == Some(name))
                };
                if let (Some(feature), Some(path), Some(_)) = (
                    field(entries, "feature_id"),
                    field(entries, "path"),
                    field(entries, "value_type"),
                ) && let Some(renamed) = entries[feature]
                    .1
                    .as_integer()
                    .and_then(|id| u64::try_from(id).ok())
                    .zip(entries[path].1.as_text())
                    .and_then(|(id, path)| rename(FeatureId(id), path))
                {
                    entries[path].1 = ciborium::Value::Text(renamed);
                }
                for (key, item) in entries {
                    visit(key, rename);
                    visit(item, rename);
                }
            }
            ciborium::Value::Array(items) => {
                for item in items {
                    visit(item, rename);
                }
            }
            ciborium::Value::Tag(_, item) => visit(item, rename),
            _ => {}
        }
    }
    let mut value = ciborium::Value::serialized(product)?;
    visit(&mut value, &rename);
    value.deserialized()
}

/// Decodes a native document written by schemas 0..={LAST_LEGACY_SCHEMA} into the product
/// model. Callers re-encode the result with the current serde codec; nothing else may use
/// this module.
pub(super) fn decode(bytes: &[u8]) -> Result<Decoded, PersistenceError> {
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(PersistenceError::InvalidMagic);
    }
    let schema = reader.u16()?;
    let mut point_profiles = BTreeSet::new();
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
            | EDGE_FINISH_SCHEMA
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
            | SPATIAL_SWEEP_PATH_SCHEMA
            | PLANAR_FACE_ATTACHMENT_SCHEMA
            | FREE_WORKPLANE_SCHEMA
            | AXIAL_ATTACHMENT_SCHEMA
            | OCCURRENCE_COLOR_SCHEMA
            | GLB_IMPORT_SCHEMA
            | DRAWING_PAGE_CONTRACT_SCHEMA
            | DRAWING_VIEW_CONTRACT_SCHEMA
            | DRAWING_SECTION_CONTRACT_SCHEMA
            | DRAWING_DETAIL_CONTRACT_SCHEMA
            | DRAWING_DIMENSION_CONTRACT_SCHEMA
            | DRAWING_TOLERANCE_CONTRACT_SCHEMA
            | DRAWING_ANNOTATION_CONTRACT_SCHEMA
            | SKETCH_PROJECTION_SCHEMA
            | SKETCH_CONSTRUCTION_SCHEMA
            | HELICAL_ASSEMBLY_JOINT_SCHEMA
            | IGES_IMPORT_SCHEMA
            | CONSTRUCTION_GEOMETRY_SCHEMA
            | CONSTRUCTION_AXIS_SCHEMA
            | CONSTRUCTION_PLANE_SCHEMA
            | CONSTRUCTION_PLANE_WORKPLANE_SCHEMA
            | LOFT_GUIDE_CONTINUITY_SCHEMA
            | SHELL_DIRECTION_SCHEMA
            | VARIABLE_FILLET_SCHEMA
            | ADVANCED_CHAMFER_SCHEMA
            | NESTED_ASSEMBLY_PATH_SCHEMA
            | NESTED_DRAWING_PATH_SCHEMA
            | NESTED_INSTANCE_TRANSFORM_SCHEMA
            | TYPED_DRAWING_DIMENSION_SCHEMA
            | DRAWING_GDT_SCHEMA
            | DRAWING_BOM_BALLOON_SCHEMA
            | SHEET_METAL_SCHEMA
            | WELDMENT_MEMBER_SCHEMA
            | WELDMENT_JOINT_SCHEMA
            | SURFACE_BODY_SCHEMA
            | SURFACE_OPERATION_SCHEMA
            | SURFACE_KNIT_SCHEMA
            | SURFACE_THICKEN_SCHEMA
            | IMPORTED_EXACT_BODY_KIND_SCHEMA
            | CAM_PLAN_SCHEMA
            | DOWEL_JOINERY_SCHEMA
            | PRODUCTION_CODE_SCHEMA
            | DOWEL_PHYSICAL_HOLE_BINDING_SCHEMA
            | ASSEMBLY_RECIPE_SCHEMA
            | PROFILE_FACE_REFERENCE_SCHEMA
            | LAST_LEGACY_SCHEMA
    ) {
        return Err(PersistenceError::UnsupportedSchema(schema));
    }
    let mut migration_losses = Vec::new();
    let (revision_id, product) = if schema >= ENVELOPE_SCHEMA {
        let manifest_length = reader.count_with_limit(MAX_MANIFEST_BYTES as u32)? as usize;
        if manifest_length != MANIFEST_BYTES || bytes.len() < HEADER_BYTES + MANIFEST_BYTES {
            return Err(PersistenceError::Legacy(LegacyError::InvalidEnvelopeLength));
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
            return Err(PersistenceError::Legacy(LegacyError::InvalidEnvelopeLength));
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
            return Err(PersistenceError::Legacy(
                LegacyError::UnsupportedEnvelopeIdentity,
            ));
        }
        if !manifest.is_finished() || !reader.is_finished() {
            return Err(PersistenceError::TrailingBytes);
        }
        let mut payload_reader = Reader::new(payload);
        let revision_id = payload_reader.u64()?;
        let product = read_product(
            &mut payload_reader,
            ProductSchemaCapabilities::current(schema),
            &mut point_profiles,
        )?;
        if !payload_reader.is_finished() {
            return Err(PersistenceError::TrailingBytes);
        }
        (revision_id, product)
    } else {
        let revision_id = reader.u64()?;
        let nodes = read_nodes(&mut reader, schema == LEGACY_SCHEMA, &mut migration_losses)?;
        let mut product = if schema == PRODUCT_SCHEMA {
            read_product(
                &mut reader,
                ProductSchemaCapabilities::PRODUCT_V2,
                &mut point_profiles,
            )?
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
    Ok(Decoded {
        point_profiles,
        revision_id,
        product,
        migration_losses,
    })
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
            return Err(PersistenceError::Legacy(LegacyError::DuplicateNode(id)));
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
            kind => return Err(PersistenceError::Legacy(LegacyError::InvalidNodeKind(kind))),
        }
        .map_err(CanonicalError::Graph)?;
        if nodes.insert(id, Arc::new(node)).is_some() {
            return Err(PersistenceError::Legacy(LegacyError::DuplicateNode(id)));
        }
    }
    Ok(nodes)
}

fn read_ports(reader: &mut Reader<'_>) -> Result<Vec<PortSpec>, PersistenceError> {
    let mut ports = Vec::new();
    for _ in 0..reader.count()? {
        let name = reader.string()?;
        if reader.u8()? != 1 {
            return Err(PersistenceError::Legacy(LegacyError::InvalidPortType));
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
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidOverrideMergePolicy,
            ));
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
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidParameterSlot(
                value,
            )));
        }
    };
    ParameterPath::new(path)
        .map_err(|_| PersistenceError::Legacy(LegacyError::InvalidParameterPath))
}

fn read_feature_parameter_target(
    reader: &mut Reader<'_>,
    general_parameter_paths: bool,
) -> Result<FeatureParameterTarget, PersistenceError> {
    let feature_id = FeatureId(reader.u64()?);
    let (path, value_type) = if general_parameter_paths {
        let path = ParameterPath::new(reader.string()?)
            .map_err(|_| PersistenceError::Legacy(LegacyError::InvalidParameterPath))?;
        let value_type = match reader.u8()? {
            1 => ParameterValueType::Length,
            2 => ParameterValueType::Angle,
            3 => ParameterValueType::Scalar,
            value => {
                return Err(PersistenceError::Legacy(
                    LegacyError::InvalidParameterValueType(value),
                ));
            }
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
        value => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidPersistentDimensionTarget(value),
            ));
        }
    };
    let unit = match reader.u8()? {
        1 => DimensionDisplayUnit::Millimetres,
        2 => DimensionDisplayUnit::Centimetres,
        3 => DimensionDisplayUnit::Inches,
        value => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidDimensionDisplayUnit(value),
            ));
        }
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
                    value => {
                        return Err(PersistenceError::Legacy(
                            LegacyError::InvalidOptionalMarker(value),
                        ));
                    }
                },
            },
            input_digest: reader.string()?,
            result_digest: reader.string()?,
            applied_value_bits: reader.u64()?,
        })),
        value => Err(PersistenceError::Legacy(
            LegacyError::InvalidOptionalMarker(value),
        )),
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
                    value => {
                        return Err(PersistenceError::Legacy(
                            LegacyError::InvalidClearanceOwner(value),
                        ));
                    }
                });
            }
            ClearanceOwner::Occurrence(path)
        }
        2 => ClearanceOwner::Space(SpaceId(reader.u64()?)),
        value => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidClearanceOwner(value),
            ));
        }
    };
    let reason = reader.string()?;
    let volume = read_bounded_volume(reader)?;
    if reader.u8()? != 1 {
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidClearanceCoordinateFrame,
        ));
    }
    let tolerance =
        TolerancePolicy::new(f64::from_bits(reader.u64()?)).map_err(CanonicalError::from)?;
    let severity = match reader.u8()? {
        1 => ClearanceSeverity::Advisory,
        2 => ClearanceSeverity::Required,
        value => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidClearanceSeverity(value),
            ));
        }
    };
    let derived_from = match reader.u8()? {
        0 => None,
        1 => Some(read_identity(reader)?),
        value => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidOptionalMarker(value),
            ));
        }
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
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidResolution(
                value,
            )));
        }
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

fn read_profile_face_reference(
    reader: &mut Reader<'_>,
) -> Result<ProfileFaceReference, PersistenceError> {
    match reader.u8()? {
        1 => Ok(ProfileFaceReference::Start),
        2 => Ok(ProfileFaceReference::End),
        3 => Ok(ProfileFaceReference::Segment {
            entity_id: reader.u64()?,
            source_name: reader.string()?,
        }),
        4 => Ok(ProfileFaceReference::NamedResult(reader.string()?)),
        value => Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
            value,
        ))),
    }
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
            value => {
                return Err(PersistenceError::Legacy(
                    LegacyError::InvalidReferenceStability(value),
                ));
            }
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
        value => Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
            value,
        ))),
    }
}

fn read_sketch_region_profile(reader: &mut Reader<'_>) -> Result<PadProfile, PersistenceError> {
    Ok(PadProfile::SketchRegion {
        sketch: FeatureId(reader.u64()?),
        region: SketchRegionId(reader.u64()?),
    })
}

/// The direction and extent of an old sketch pad or sketch pocket; files before feature
/// extents store only a normal side and a blind distance.
fn read_sketch_feature_extent(
    reader: &mut Reader<'_>,
    capabilities: ProductSchemaCapabilities,
) -> Result<(FeatureDirection, FeatureExtent), PersistenceError> {
    if capabilities.feature_extents {
        return Ok((
            read_feature_direction(reader)?,
            read_feature_extent(reader)?,
        ));
    }
    let direction = match reader.u8()? {
        1 => FeatureDirection::AlongNormal,
        2 => FeatureDirection::OppositeNormal,
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                value,
            )));
        }
    };
    Ok((
        direction,
        FeatureExtent::Blind(read_extent_dimension(reader)?),
    ))
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
        value => Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
            value,
        ))),
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
        value => Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
            value,
        ))),
    }
}

fn read_import_receipt(
    reader: &mut Reader<'_>,
    sketchup_scene: bool,
    glb_import: bool,
    iges_import: bool,
) -> Result<ImportReceipt, PersistenceError> {
    let id = ImportId(reader.u64()?);
    let format = match reader.u8()? {
        1 => ImportFormat::Stl,
        2 => ImportFormat::Dxf,
        3 => ImportFormat::Step,
        4 if sketchup_scene => ImportFormat::SketchupScene,
        5 if glb_import => ImportFormat::Glb,
        6 if iges_import => ImportFormat::Iges,
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidImportFormat(
                value,
            )));
        }
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
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidImportUnit(
                value,
            )));
        }
    };
    let authority = match reader.u8()? {
        1 => ImportUnitAuthority::FileDeclared,
        2 => ImportUnitAuthority::UserDeclared,
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidImportUnit(
                value,
            )));
        }
    };
    let parser_id = reader.string()?;
    let parser_version = reader.string()?;
    let mut diagnostics = Vec::new();
    for _ in 0..reader.count_with_limit(MAX_IMPORT_DIAGNOSTICS as u32)? {
        let severity = match reader.u8()? {
            1 => ImportDiagnosticSeverity::Info,
            2 => ImportDiagnosticSeverity::Warning,
            value => {
                return Err(PersistenceError::Legacy(
                    LegacyError::InvalidImportDiagnostic(value),
                ));
            }
        };
        let code = reader.string()?;
        let subject = match reader.u8()? {
            0 => None,
            1 => Some(reader.string()?),
            value => {
                return Err(PersistenceError::Legacy(
                    LegacyError::InvalidOptionalMarker(value),
                ));
            }
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
            4 if glb_import => ImportOutputRef::Group(GroupId(reader.u64()?)),
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidImportOutput(
                    value,
                )));
            }
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

fn read_workplane(
    reader: &mut Reader<'_>,
    free_workplanes: bool,
    construction_plane_workplanes: bool,
) -> Result<WorkplaneSpec, PersistenceError> {
    let support = match reader.u8()? {
        4 if free_workplanes => WorkplaneSupport::Free,
        1 => WorkplaneSupport::Principal(match reader.u8()? {
            1 => PrincipalPlane::Xy,
            2 => PrincipalPlane::Yz,
            3 => PrincipalPlane::Xz,
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                    value,
                )));
            }
        }),
        2 => WorkplaneSupport::Offset {
            base: FeatureId(reader.u64()?),
            distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
        },
        5 if construction_plane_workplanes => WorkplaneSupport::ConstructionPlane {
            feature: FeatureId(reader.u64()?),
        },
        3 => WorkplaneSupport::PlanarFace {
            reference: Box::new(read_exact_reference(reader)?),
            health: match reader.u8()? {
                1 => WorkplaneSupportHealth::Resolved,
                2 => WorkplaneSupportHealth::Ambiguous,
                3 => WorkplaneSupportHealth::Lost,
                4 => WorkplaneSupportHealth::Stale,
                value => {
                    return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                        value,
                    )));
                }
            },
        },
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                value,
            )));
        }
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
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                    value,
                )));
            }
        },
    })
}

fn read_sketch(
    reader: &mut Reader<'_>,
    full_constraint_vocabulary: bool,
    cubic_bezier_sketch: bool,
    sketch_projection: bool,
    sketch_construction: bool,
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
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                    value,
                )));
            }
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
            17 if sketch_projection => {
                let entity = SketchEntityId(reader.u64()?);
                let source_feature = FeatureId(reader.u64()?);
                let source_entity = SketchEntityId(reader.u64()?);
                let target = entities
                    .iter()
                    .find(|candidate| candidate.id() == entity)
                    .cloned()
                    .ok_or(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                        17,
                    )))?;
                SketchConstraintKind::Projection {
                    entity,
                    source_feature,
                    source_entity,
                    target: Box::new(target),
                }
            }
            18 if sketch_construction => SketchConstraintKind::Construction {
                entity: SketchEntityId(reader.u64()?),
            },
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                    value,
                )));
            }
        };
        constraints.push(SketchConstraint { id, kind });
    }
    Ok(SketchSpec {
        workplane,
        entities,
        constraints,
    })
}

fn read_instance_path(reader: &mut Reader<'_>) -> Result<InstancePath, PersistenceError> {
    let mut path = InstancePath::root(OccurrenceId(reader.u64()?));
    for _ in 0..reader.count_with_limit(256)? {
        path = path.with_step(match reader.u8()? {
            1 => InstancePathStep::Group(LocalGroupId(reader.u64()?)),
            2 => InstancePathStep::Occurrence(LocalOccurrenceId(reader.u64()?)),
            _ => {
                return Err(PersistenceError::InvalidCanonicalData(
                    CanonicalError::InvalidInstancePath,
                ));
            }
        });
    }
    Ok(path)
}

fn read_assembly_mate(
    reader: &mut Reader<'_>,
    typed_attachments: bool,
    axial_attachments: bool,
    nested_assembly_paths: bool,
) -> Result<AssemblyMate, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_MATE_SCHEMA_V1 {
        return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate));
    }
    let id = AssemblyMateId(reader.u64()?);
    let mut endpoint = || -> Result<AssemblyMateEndpoint, PersistenceError> {
        let instance_path = if nested_assembly_paths {
            read_instance_path(reader)?
        } else {
            InstancePath::root(OccurrenceId(reader.u64()?))
        };
        let attachment = if typed_attachments {
            match reader.u8()? {
                1 => AssemblyMateAttachment::ReferenceOnly(read_exact_reference(reader)?),
                2 => {
                    let reference = read_exact_reference(reader)?;
                    let mut origin = [0.0; 3];
                    let mut normal = [0.0; 3];
                    for value in &mut origin {
                        *value = f64::from_bits(reader.u64()?);
                    }
                    for value in &mut normal {
                        *value = f64::from_bits(reader.u64()?);
                    }
                    AssemblyMateAttachment::PlanarFace(
                        PlanarFaceAttachment::new(reference, origin, normal)
                            .ok_or(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate))?,
                    )
                }
                3 if axial_attachments => {
                    let reference = read_exact_reference(reader)?;
                    let kind = match reader.u8()? {
                        1 => AxialAttachmentKind::Axis,
                        2 => AxialAttachmentKind::CylindricalFace,
                        _ => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate));
                        }
                    };
                    let mut origin = [0.0; 3];
                    let mut direction = [0.0; 3];
                    for value in &mut origin {
                        *value = f64::from_bits(reader.u64()?);
                    }
                    for value in &mut direction {
                        *value = f64::from_bits(reader.u64()?);
                    }
                    AssemblyMateAttachment::Axial(
                        AxialAttachment::new(reference, kind, origin, direction)
                            .ok_or(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate))?,
                    )
                }
                _ => return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate)),
            }
        } else {
            AssemblyMateAttachment::ReferenceOnly(read_exact_reference(reader)?)
        };
        let health = match reader.u8()? {
            1 => AssemblyReferenceHealth::Resolved,
            2 => AssemblyReferenceHealth::Ambiguous {
                candidate_count: reader.u32()?,
            },
            3 => AssemblyReferenceHealth::Lost,
            4 => AssemblyReferenceHealth::Broken,
            _ => return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate)),
        };
        Ok(AssemblyMateEndpoint {
            instance_path,
            attachment,
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
        _ => return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyMate)),
    };
    Ok(AssemblyMate {
        schema,
        id,
        endpoint_a: Box::new(endpoint_a),
        endpoint_b: Box::new(endpoint_b),
        kind,
    })
}

fn read_assembly_joint(
    reader: &mut Reader<'_>,
    allow_helical: bool,
    nested_assembly_paths: bool,
) -> Result<AssemblyJoint, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_JOINT_SCHEMA_V1 {
        return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyJoint));
    }
    let id = AssemblyJointId(reader.u64()?);
    let parent_instance_path = if nested_assembly_paths {
        read_instance_path(reader)?
    } else {
        InstancePath::root(OccurrenceId(reader.u64()?))
    };
    let child_instance_path = if nested_assembly_paths {
        read_instance_path(reader)?
    } else {
        InstancePath::root(OccurrenceId(reader.u64()?))
    };
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
        4 if allow_helical => AssemblyJointKind::Helical {
            axis: read_assembly_joint_axis(reader)?,
            limits: read_assembly_joint_limits(reader)?,
            lead_mm_per_revolution: f64::from_bits(reader.u64()?),
            position_degrees: f64::from_bits(reader.u64()?),
        },
        _ => return Err(PersistenceError::Legacy(LegacyError::InvalidAssemblyJoint)),
    };
    Ok(AssemblyJoint {
        schema,
        id,
        parent_instance_path,
        child_instance_path,
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
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidMechanicalInterface,
        ));
    }
    let id = MechanicalInterfaceId(reader.u64()?);
    let occurrence_id = OccurrenceId(reader.u64()?);
    let role = match reader.u8()? {
        1 => MechanicalRole::Mounting,
        2 => MechanicalRole::Support,
        3 => MechanicalRole::Guide,
        _ => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidMechanicalInterface,
            ));
        }
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
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidMechanicalInterface,
        ));
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
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidMechanicalCondition,
        ));
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
                _ => {
                    return Err(PersistenceError::Legacy(
                        LegacyError::InvalidMechanicalCondition,
                    ));
                }
            },
            tolerance_degrees: f64::from_bits(reader.u64()?),
        },
        4 => MechanicalConditionKind::JointTravel {
            joint_id: AssemblyJointId(reader.u64()?),
            minimum: f64::from_bits(reader.u64()?),
            maximum: f64::from_bits(reader.u64()?),
        },
        _ => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidMechanicalCondition,
            ));
        }
    };
    let condition = MechanicalCondition::new(id, kind);
    if !condition.has_valid_shape() {
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidMechanicalCondition,
        ));
    }
    Ok(condition)
}

fn read_assembly_motion_coupling(
    reader: &mut Reader<'_>,
) -> Result<AssemblyMotionCoupling, PersistenceError> {
    let schema = reader.string()?;
    if schema != ASSEMBLY_MOTION_COUPLING_SCHEMA_V1 {
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidAssemblyMotionCoupling,
        ));
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
                _ => {
                    return Err(PersistenceError::Legacy(
                        LegacyError::InvalidAssemblyMotionCoupling,
                    ));
                }
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
                _ => {
                    return Err(PersistenceError::Legacy(
                        LegacyError::InvalidAssemblyMotionCoupling,
                    ));
                }
            },
        },
        5 => AssemblyTransmissionKind::LeadScrew {
            lead_mm_per_revolution: f64::from_bits(reader.u64()?),
            handedness: match reader.u8()? {
                1 => ScrewHandedness::Right,
                2 => ScrewHandedness::Left,
                _ => {
                    return Err(PersistenceError::Legacy(
                        LegacyError::InvalidAssemblyMotionCoupling,
                    ));
                }
            },
        },
        _ => {
            return Err(PersistenceError::Legacy(
                LegacyError::InvalidAssemblyMotionCoupling,
            ));
        }
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
        return Err(PersistenceError::Legacy(
            LegacyError::InvalidAssemblyMotionStudy,
        ));
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

#[allow(clippy::too_many_arguments)]
fn read_drawing_sheet_with_annotations(
    reader: &mut Reader<'_>,
    page_contract: bool,
    view_contract: bool,
    section_contract: bool,
    detail_contract: bool,
    dimension_contract: bool,
    extended_contracts: (bool, bool),
    nested_drawing_paths: bool,
    typed_drawing_dimensions: bool,
    drawing_gdt: bool,
    drawing_bom_balloons: bool,
) -> Result<DrawingSheet, PersistenceError> {
    let (tolerance_contract, annotation_contract) = extended_contracts;
    let persisted_schema = reader.string()?;
    let expected_schema = if page_contract {
        ORTHOGRAPHIC_DRAWING_SCHEMA_V2
    } else {
        ORTHOGRAPHIC_DRAWING_SCHEMA_V1
    };
    if persisted_schema != expected_schema {
        return Err(invalid_drawing_sheet());
    }
    let id = DrawingSheetId(reader.u64()?);
    let name = reader.string()?;
    let source = match reader.u8()? {
        1 => DrawingSource::Definition(DefinitionId(reader.u64()?)),
        2 => DrawingSource::RigidAssembly {
            occurrence_ids: read_ids(reader)?.into_iter().map(OccurrenceId).collect(),
        },
        3 if nested_drawing_paths => {
            let mut instance_paths = Vec::new();
            for _ in 0..reader.count_with_limit(MAX_COLLECTION_ITEMS)? {
                instance_paths.push(read_instance_path(reader)?);
            }
            DrawingSource::RigidAssemblyInstances { instance_paths }
        }
        _ => return Err(invalid_drawing_sheet()),
    };
    if !page_contract {
        return DrawingSheet::new(id, name, source).map_err(|error| {
            PersistenceError::InvalidCanonicalData(CanonicalError::Drawing(error))
        });
    }
    let size = match reader.u8()? {
        1 => DrawingPageSize::A0,
        2 => DrawingPageSize::A1,
        3 => DrawingPageSize::A2,
        4 => DrawingPageSize::A3,
        5 => DrawingPageSize::A4,
        _ => return Err(invalid_drawing_sheet()),
    };
    let orientation = match reader.u8()? {
        1 => DrawingPageOrientation::Portrait,
        2 => DrawingPageOrientation::Landscape,
        _ => return Err(invalid_drawing_sheet()),
    };
    let scale_numerator = reader.u32()?;
    let scale_denominator = reader.u32()?;
    let scale = DrawingScale::new(scale_numerator, scale_denominator)
        .map_err(|_| invalid_drawing_sheet())?;
    if scale.numerator() != scale_numerator || scale.denominator() != scale_denominator {
        return Err(invalid_drawing_sheet());
    }
    let page = DrawingPageTemplate::new(
        size,
        orientation,
        scale,
        DrawingMargins::new(reader.u16()?, reader.u16()?, reader.u16()?, reader.u16()?),
    )
    .map_err(|_| invalid_drawing_sheet())?;
    let title_block = DrawingTitleBlock::new(
        reader.string()?,
        reader.string()?,
        reader.string()?,
        reader.string()?,
    )
    .map_err(|_| invalid_drawing_sheet())?;
    if !view_contract {
        return DrawingSheet::with_contract(id, name, source, page, title_block).map_err(|error| {
            PersistenceError::InvalidCanonicalData(CanonicalError::Drawing(error))
        });
    }
    let view_count = reader.count_with_limit(MAX_DRAWING_VIEWS as u32)?;
    let mut views = Vec::with_capacity(view_count as usize);
    for _ in 0..view_count {
        let view = match reader.u8()? {
            1 => OrthographicViewKind::Front,
            2 => OrthographicViewKind::Top,
            3 => OrthographicViewKind::Right,
            4 => OrthographicViewKind::Isometric,
            5 => {
                let mut components = [0.0; 9];
                for component in &mut components {
                    *component = f64::from_bits(reader.u64()?);
                }
                OrthographicViewKind::Auxiliary(
                    DrawingViewFrame::from_persisted_axes(
                        [components[0], components[1], components[2]],
                        [components[3], components[4], components[5]],
                        [components[6], components[7], components[8]],
                    )
                    .map_err(|_| invalid_drawing_sheet())?,
                )
            }
            6 if section_contract => {
                let mut components = [0.0; 9];
                for component in &mut components {
                    *component = f64::from_bits(reader.u64()?);
                }
                let frame = DrawingViewFrame::from_persisted_axes(
                    [components[0], components[1], components[2]],
                    [components[3], components[4], components[5]],
                    [components[6], components[7], components[8]],
                )
                .map_err(|_| invalid_drawing_sheet())?;
                OrthographicViewKind::Section(
                    DrawingSectionPlane::new(frame, f64::from_bits(reader.u64()?))
                        .map_err(|_| invalid_drawing_sheet())?,
                )
            }
            7 if detail_contract => {
                let mut components = [0.0; 9];
                for component in &mut components {
                    *component = f64::from_bits(reader.u64()?);
                }
                let frame = DrawingViewFrame::from_persisted_axes(
                    [components[0], components[1], components[2]],
                    [components[3], components[4], components[5]],
                    [components[6], components[7], components[8]],
                )
                .map_err(|_| invalid_drawing_sheet())?;
                let center_mm = [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)];
                let radius_mm = f64::from_bits(reader.u64()?);
                let numerator = reader.u32()?;
                let denominator = reader.u32()?;
                let magnification = DrawingScale::new(numerator, denominator)
                    .map_err(|_| invalid_drawing_sheet())?;
                if magnification.numerator() != numerator
                    || magnification.denominator() != denominator
                {
                    return Err(invalid_drawing_sheet());
                }
                OrthographicViewKind::Detail(
                    DrawingDetailRegion::new(frame, center_mm, radius_mm, magnification)
                        .map_err(|_| invalid_drawing_sheet())?,
                )
            }
            _ => return Err(invalid_drawing_sheet()),
        };
        views.push(view);
    }
    if !dimension_contract {
        return DrawingSheet::with_contract_and_views(id, name, source, page, title_block, views)
            .map_err(|error| {
                PersistenceError::InvalidCanonicalData(CanonicalError::Drawing(error))
            });
    }
    let dimension_count = reader.count_with_limit(MAX_DRAWING_DIMENSIONS as u32)?;
    let mut linear_dimensions = Vec::with_capacity(dimension_count as usize);
    for _ in 0..dimension_count {
        let dimension_id = DrawingDimensionId(reader.u64()?);
        let view_stable_name = reader.string()?;
        let source_line_id = reader.string()?;
        let offset_page_mm = f64::from_bits(reader.u64()?);
        let tolerance = match reader.u8()? {
            0 => DrawingDimensionTolerance::None,
            1 if tolerance_contract => {
                DrawingDimensionTolerance::symmetric(f64::from_bits(reader.u64()?))
                    .map_err(|_| invalid_drawing_sheet())?
            }
            2 if tolerance_contract => DrawingDimensionTolerance::bilateral(
                f64::from_bits(reader.u64()?),
                f64::from_bits(reader.u64()?),
            )
            .map_err(|_| invalid_drawing_sheet())?,
            _ => return Err(invalid_drawing_sheet()),
        };
        linear_dimensions.push(
            DrawingLinearDimension::from_persisted(
                dimension_id,
                view_stable_name,
                source_line_id,
                offset_page_mm,
                tolerance,
            )
            .map_err(|_| invalid_drawing_sheet())?,
        );
    }
    let (title_block, notes) = if annotation_contract {
        let parametric = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid_drawing_sheet()),
        };
        let title_block = DrawingTitleBlock::from_persisted(
            title_block.title().to_owned(),
            title_block.drawing_number().to_owned(),
            title_block.revision().to_owned(),
            title_block.author().to_owned(),
            parametric,
        )
        .map_err(|_| invalid_drawing_sheet())?;
        let note_count = reader.count_with_limit(MAX_DRAWING_NOTES as u32)?;
        let mut notes = Vec::with_capacity(note_count as usize);
        for _ in 0..note_count {
            notes.push(
                DrawingNote::new(
                    DrawingNoteId(reader.u64()?),
                    [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)],
                    reader.string()?,
                )
                .map_err(|_| invalid_drawing_sheet())?,
            );
        }
        (title_block, notes)
    } else {
        (title_block, Vec::new())
    };
    let (angular_dimensions, circular_dimensions) = if typed_drawing_dimensions {
        let angular_count = reader.count_with_limit(MAX_DRAWING_DIMENSIONS as u32)?;
        let mut angular_dimensions = Vec::with_capacity(angular_count as usize);
        for _ in 0..angular_count {
            angular_dimensions.push(
                DrawingAngularDimension::from_persisted(
                    DrawingDimensionId(reader.u64()?),
                    reader.string()?,
                    [reader.string()?, reader.string()?],
                    f64::from_bits(reader.u64()?),
                    read_drawing_tolerance(reader)?,
                )
                .map_err(|_| invalid_drawing_sheet())?,
            );
        }
        let circular_count = reader.count_with_limit(MAX_DRAWING_DIMENSIONS as u32)?;
        let mut circular_dimensions = Vec::with_capacity(circular_count as usize);
        for _ in 0..circular_count {
            let dimension_id = DrawingDimensionId(reader.u64()?);
            let view_stable_name = reader.string()?;
            let source_circle_id = reader.string()?;
            let kind = match reader.u8()? {
                1 => DrawingCircularDimensionKind::Radius,
                2 => DrawingCircularDimensionKind::Diameter,
                _ => return Err(invalid_drawing_sheet()),
            };
            circular_dimensions.push(
                DrawingCircularDimension::from_persisted(
                    dimension_id,
                    view_stable_name,
                    source_circle_id,
                    kind,
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    read_drawing_tolerance(reader)?,
                )
                .map_err(|_| invalid_drawing_sheet())?,
            );
        }
        (angular_dimensions, circular_dimensions)
    } else {
        (Vec::new(), Vec::new())
    };
    let (datum_symbols, feature_control_frames) = if drawing_gdt {
        let datum_count = reader.count_with_limit(MAX_DRAWING_NOTES as u32)?;
        let mut datum_symbols = Vec::with_capacity(datum_count as usize);
        for _ in 0..datum_count {
            datum_symbols.push(
                DrawingDatumSymbol::from_persisted(
                    DrawingDatumId(reader.u64()?),
                    reader.string()?,
                    reader.string()?,
                    reader.string()?,
                    [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)],
                )
                .map_err(|_| invalid_drawing_sheet())?,
            );
        }
        let frame_count = reader.count_with_limit(MAX_DRAWING_NOTES as u32)?;
        let mut feature_control_frames = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            let frame_id = DrawingFeatureControlFrameId(reader.u64()?);
            let view_stable_name = reader.string()?;
            let source_line_id = reader.string()?;
            let characteristic = read_drawing_characteristic(reader.u8()?)?;
            let tolerance_mm = f64::from_bits(reader.u64()?);
            let diameter_zone = match reader.u8()? {
                0 => false,
                1 => true,
                _ => return Err(invalid_drawing_sheet()),
            };
            let material_condition = read_drawing_material_condition(reader.u8()?)?;
            let reference_count = reader.count_with_limit(3)?;
            let mut references = Vec::with_capacity(reference_count as usize);
            for _ in 0..reference_count {
                references.push(
                    DrawingDatumReference::new(
                        reader.string()?,
                        read_drawing_material_condition(reader.u8()?)?,
                    )
                    .map_err(|_| invalid_drawing_sheet())?,
                );
            }
            feature_control_frames.push(
                DrawingFeatureControlFrame::from_persisted(
                    frame_id,
                    view_stable_name,
                    source_line_id,
                    characteristic,
                    tolerance_mm,
                    diameter_zone,
                    material_condition,
                    references,
                    [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)],
                )
                .map_err(|_| invalid_drawing_sheet())?,
            );
        }
        (datum_symbols, feature_control_frames)
    } else {
        (Vec::new(), Vec::new())
    };
    let bom_balloons = if drawing_bom_balloons {
        let balloon_count = reader.count_with_limit(MAX_DRAWING_BOM_BALLOONS as u32)?;
        let mut bom_balloons = Vec::with_capacity(balloon_count as usize);
        for _ in 0..balloon_count {
            bom_balloons.push(
                DrawingBomBalloon::from_persisted(
                    DrawingBomBalloonId(reader.u64()?),
                    reader.string()?,
                    read_instance_path(reader)?,
                    reader.u32()?,
                    [f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)],
                )
                .map_err(|_| invalid_drawing_sheet())?,
            );
        }
        bom_balloons
    } else {
        Vec::new()
    };
    DrawingSheet::with_contract_views_and_annotations(
        id,
        name,
        source,
        page,
        title_block,
        views,
        DrawingAnnotations::with_bom_annotations(
            linear_dimensions,
            angular_dimensions,
            circular_dimensions,
            datum_symbols,
            feature_control_frames,
            bom_balloons,
            notes,
        ),
    )
    .map_err(|error| PersistenceError::InvalidCanonicalData(CanonicalError::Drawing(error)))
}

fn read_drawing_characteristic(
    code: u8,
) -> Result<DrawingGeometricCharacteristic, PersistenceError> {
    match code {
        1 => Ok(DrawingGeometricCharacteristic::Straightness),
        2 => Ok(DrawingGeometricCharacteristic::Flatness),
        3 => Ok(DrawingGeometricCharacteristic::Circularity),
        4 => Ok(DrawingGeometricCharacteristic::Cylindricity),
        5 => Ok(DrawingGeometricCharacteristic::ProfileOfLine),
        6 => Ok(DrawingGeometricCharacteristic::ProfileOfSurface),
        7 => Ok(DrawingGeometricCharacteristic::Angularity),
        8 => Ok(DrawingGeometricCharacteristic::Perpendicularity),
        9 => Ok(DrawingGeometricCharacteristic::Parallelism),
        10 => Ok(DrawingGeometricCharacteristic::Position),
        11 => Ok(DrawingGeometricCharacteristic::Concentricity),
        12 => Ok(DrawingGeometricCharacteristic::Symmetry),
        13 => Ok(DrawingGeometricCharacteristic::CircularRunout),
        14 => Ok(DrawingGeometricCharacteristic::TotalRunout),
        _ => Err(invalid_drawing_sheet()),
    }
}

fn read_drawing_material_condition(code: u8) -> Result<DrawingMaterialCondition, PersistenceError> {
    match code {
        0 => Ok(DrawingMaterialCondition::None),
        1 => Ok(DrawingMaterialCondition::MaximumMaterial),
        2 => Ok(DrawingMaterialCondition::LeastMaterial),
        3 => Ok(DrawingMaterialCondition::RegardlessOfFeatureSize),
        _ => Err(invalid_drawing_sheet()),
    }
}

fn read_drawing_tolerance(
    reader: &mut Reader<'_>,
) -> Result<DrawingDimensionTolerance, PersistenceError> {
    match reader.u8()? {
        0 => Ok(DrawingDimensionTolerance::None),
        1 => DrawingDimensionTolerance::symmetric(f64::from_bits(reader.u64()?))
            .map_err(|_| invalid_drawing_sheet()),
        2 => DrawingDimensionTolerance::bilateral(
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
        )
        .map_err(|_| invalid_drawing_sheet()),
        _ => Err(invalid_drawing_sheet()),
    }
}

fn read_cam_plan(reader: &mut Reader<'_>) -> Result<CamPlan, PersistenceError> {
    let point = |reader: &mut Reader<'_>| -> Result<[f64; 3], PersistenceError> {
        Ok([
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
        ])
    };
    let id = CamPlanId(reader.u64()?);
    let name = reader.string()?;
    let units = match reader.u8()? {
        1 => CamUnits::Millimetres,
        value => {
            return Err(PersistenceError::Legacy(LegacyError::UnsupportedUnits(
                value,
            )));
        }
    };
    let target = CamTarget {
        definition_id: DefinitionId(reader.u64()?),
        feature_id: FeatureId(reader.u64()?),
        exact_graph_digest: reader.string()?,
    };
    let stock = CamStock {
        minimum_mm: point(reader)?,
        maximum_mm: point(reader)?,
    };
    let number = reader.u32()?;
    let kind = match reader.u8()? {
        1 => CamToolKind::FlatEndMill,
        2 => CamToolKind::BallEndMill,
        3 => CamToolKind::Drill,
        _ => {
            return Err(PersistenceError::InvalidCanonicalData(CanonicalError::Cam(
                crate::cam::CamError::InvalidPlan,
            )));
        }
    };
    let tool = CamTool {
        number,
        kind,
        diameter_mm: f64::from_bits(reader.u64()?),
        flute_length_mm: f64::from_bits(reader.u64()?),
        overall_length_mm: f64::from_bits(reader.u64()?),
        holder_diameter_mm: f64::from_bits(reader.u64()?),
        holder_length_mm: f64::from_bits(reader.u64()?),
        spindle_rpm: reader.u32()?,
        feed_mm_per_min: f64::from_bits(reader.u64()?),
        plunge_mm_per_min: f64::from_bits(reader.u64()?),
    };
    let work_offset = match reader.u8()? {
        54 => CamWorkOffset::G54,
        55 => CamWorkOffset::G55,
        56 => CamWorkOffset::G56,
        57 => CamWorkOffset::G57,
        58 => CamWorkOffset::G58,
        59 => CamWorkOffset::G59,
        _ => {
            return Err(PersistenceError::InvalidCanonicalData(CanonicalError::Cam(
                crate::cam::CamError::InvalidPlan,
            )));
        }
    };
    let setup = CamSetup {
        work_offset,
        origin_mm: point(reader)?,
        x_axis: point(reader)?,
        y_axis: point(reader)?,
        safe_height_mm: f64::from_bits(reader.u64()?),
    };
    let cut_parameters = CamCutParameters {
        maximum_stepdown_mm: f64::from_bits(reader.u64()?),
        stepover_ratio: f64::from_bits(reader.u64()?),
        radial_allowance_mm: f64::from_bits(reader.u64()?),
        axial_allowance_mm: f64::from_bits(reader.u64()?),
    };
    Ok(CamPlan::from_parts(
        id,
        name,
        units,
        target,
        stock,
        tool,
        setup,
        cut_parameters,
    ))
}

fn read_dowel_joint(
    reader: &mut Reader<'_>,
    read_physical_hole_bindings: bool,
    read_pair_offsets: bool,
) -> Result<DowelJointContract, PersistenceError> {
    let id = DowelJointId(reader.u64()?);
    let name = reader.string()?;
    let point3 = |reader: &mut Reader<'_>| -> Result<[f64; 3], PersistenceError> {
        Ok([
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
            f64::from_bits(reader.u64()?),
        ])
    };
    let read_side = |reader: &mut Reader<'_>| -> Result<DowelJointFace, PersistenceError> {
        Ok(DowelJointFace {
            instance_path: read_instance_path(reader)?,
            face_origin_local_mm: point3(reader)?,
            inward_unit_local: point3(reader)?,
            bounds_min_local_mm: point3(reader)?,
            bounds_max_local_mm: point3(reader)?,
        })
    };
    let first = read_side(reader)?;
    let second = read_side(reader)?;
    let first_center_local_mm = point3(reader)?;
    let row_unit_first_local = point3(reader)?;
    let count = reader.u32()?;
    let spacing_mm = f64::from_bits(reader.u64()?);
    let dowel = DowelSpec {
        diameter_mm: f64::from_bits(reader.u64()?),
        length_mm: f64::from_bits(reader.u64()?),
        first_insertion_mm: f64::from_bits(reader.u64()?),
        second_insertion_mm: f64::from_bits(reader.u64()?),
        bottom_clearance_mm: f64::from_bits(reader.u64()?),
    };
    let pair_offsets_first_local_mm = if read_pair_offsets {
        let count = reader.count_with_limit(128)?;
        (0..count)
            .map(|_| point3(reader))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let physical_hole_pairs = if read_physical_hole_bindings && reader.u8()? != 0 {
        let count = reader.count_with_limit(MAX_COLLECTION_ITEMS)?;
        let mut bindings = Vec::with_capacity(count as usize);
        for _ in 0..count {
            bindings.push(DowelPhysicalHolePair {
                first_pocket_feature_id: FeatureId(reader.u64()?),
                second_pocket_feature_id: FeatureId(reader.u64()?),
            });
        }
        Some(bindings)
    } else {
        None
    };
    Ok(DowelJointContract {
        id,
        name,
        first,
        second,
        first_center_local_mm,
        row_unit_first_local,
        count,
        spacing_mm,
        dowel,
        pair_offsets_first_local_mm,
        physical_hole_pairs,
    })
}

fn read_recipe_key(reader: &mut Reader<'_>) -> Result<RecipeKey, PersistenceError> {
    RecipeKey::new(reader.string()?).map_err(|error| {
        PersistenceError::InvalidCanonicalData(CanonicalError::AssemblyRecipe(error))
    })
}

fn read_assembly_recipe(reader: &mut Reader<'_>) -> Result<AssemblyRecipe, PersistenceError> {
    let schema = reader.string()?;
    let key = read_recipe_key(reader)?;
    let mut parts = BTreeMap::new();
    for _ in 0..reader.count_with_limit(16_384)? {
        let part_key = read_recipe_key(reader)?;
        let instance_path = read_instance_path(reader)?;
        let definition_id = DefinitionId(reader.u64()?);
        let placement = reader.transform()?;
        let mobility = match reader.u8()? {
            1 => RecipePartMobility::Fixed,
            2 => RecipePartMobility::Movable,
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidRecipeValue(
                    value,
                )));
            }
        };
        let edit_scope = match reader.u8()? {
            1 => RecipeEditScope::Occurrence(read_instance_path(reader)?),
            2 => RecipeEditScope::SharedDefinition(DefinitionId(reader.u64()?)),
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidRecipeValue(
                    value,
                )));
            }
        };
        let mut parameters = BTreeMap::new();
        for _ in 0..reader.count_with_limit(16_384)? {
            let parameter_key = read_recipe_key(reader)?;
            let value = f64::from_bits(reader.u64()?);
            let unit = match reader.u8()? {
                1 => RecipeParameterUnit::Millimetres,
                2 => RecipeParameterUnit::Degrees,
                3 => RecipeParameterUnit::Scalar,
                value => {
                    return Err(PersistenceError::Legacy(LegacyError::InvalidRecipeValue(
                        value,
                    )));
                }
            };
            let target = if reader.boolean()? {
                let feature_id = FeatureId(reader.u64()?);
                let path = ParameterPath::new(reader.string()?)
                    .map_err(|_| PersistenceError::Legacy(LegacyError::InvalidParameterPath))?;
                let value_type = match reader.u8()? {
                    1 => ParameterValueType::Length,
                    2 => ParameterValueType::Angle,
                    3 => ParameterValueType::Scalar,
                    value => {
                        return Err(PersistenceError::Legacy(
                            LegacyError::InvalidParameterValueType(value),
                        ));
                    }
                };
                Some(FeatureParameterTarget {
                    feature_id,
                    path,
                    value_type,
                })
            } else {
                None
            };
            if parameters
                .insert(
                    parameter_key.clone(),
                    RecipeParameter {
                        value,
                        unit,
                        target,
                    },
                )
                .is_some()
            {
                return Err(PersistenceError::InvalidCanonicalData(
                    CanonicalError::AssemblyRecipe(
                        crate::assembly_recipe::AssemblyRecipeError::DuplicateKey(parameter_key),
                    ),
                ));
            }
        }
        let part = RecipePart {
            key: part_key.clone(),
            instance_path,
            definition_id,
            placement,
            mobility,
            edit_scope,
            parameters,
        };
        if parts.insert(part_key.clone(), part).is_some() {
            return Err(PersistenceError::InvalidCanonicalData(
                CanonicalError::AssemblyRecipe(
                    crate::assembly_recipe::AssemblyRecipeError::DuplicateKey(part_key),
                ),
            ));
        }
    }
    let mut relations = BTreeMap::new();
    for _ in 0..reader.count_with_limit(16_384)? {
        let relation_key = read_recipe_key(reader)?;
        let kind = match reader.u8()? {
            1 => RecipeRelationKind::Contact,
            2 => RecipeRelationKind::Coincident,
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidRecipeValue(
                    value,
                )));
            }
        };
        let first = RecipeFaceRef {
            part: read_recipe_key(reader)?,
            role: reader.string()?,
        };
        let second = RecipeFaceRef {
            part: read_recipe_key(reader)?,
            role: reader.string()?,
        };
        let relation = RecipeRelation {
            key: relation_key.clone(),
            kind,
            first,
            second,
        };
        if relations.insert(relation_key.clone(), relation).is_some() {
            return Err(PersistenceError::InvalidCanonicalData(
                CanonicalError::AssemblyRecipe(
                    crate::assembly_recipe::AssemblyRecipeError::DuplicateKey(relation_key),
                ),
            ));
        }
    }
    let mut joinery = BTreeMap::new();
    for _ in 0..reader.count_with_limit(16_384)? {
        let joinery_key = read_recipe_key(reader)?;
        let item = RecipeJoinery {
            key: joinery_key.clone(),
            first_part: read_recipe_key(reader)?,
            second_part: read_recipe_key(reader)?,
            dowel_joint_id: DowelJointId(reader.u64()?),
        };
        if joinery.insert(joinery_key.clone(), item).is_some() {
            return Err(PersistenceError::InvalidCanonicalData(
                CanonicalError::AssemblyRecipe(
                    crate::assembly_recipe::AssemblyRecipeError::DuplicateKey(joinery_key),
                ),
            ));
        }
    }
    let mut owned_features = BTreeMap::new();
    for _ in 0..reader.count_with_limit(16_384)? {
        let owned_key = read_recipe_key(reader)?;
        let owned = RecipeOwnedFeature {
            key: owned_key.clone(),
            part: read_recipe_key(reader)?,
            feature_id: FeatureId(reader.u64()?),
            kind: match reader.u8()? {
                1 => RecognizedRecipeFeatureKind::Profile,
                2 | 3 | 6 | 7 => RecognizedRecipeFeatureKind::Pad,
                4 => RecognizedRecipeFeatureKind::Workplane,
                5 => RecognizedRecipeFeatureKind::Sketch,
                value => {
                    return Err(PersistenceError::Legacy(LegacyError::InvalidRecipeValue(
                        value,
                    )));
                }
            },
            canonical_fingerprint: reader.string()?,
        };
        if owned_features.insert(owned_key.clone(), owned).is_some() {
            return Err(PersistenceError::InvalidCanonicalData(
                CanonicalError::AssemblyRecipe(
                    crate::assembly_recipe::AssemblyRecipeError::DuplicateKey(owned_key),
                ),
            ));
        }
    }
    AssemblyRecipe::new(schema, key, parts, relations, joinery, owned_features).map_err(|error| {
        PersistenceError::InvalidCanonicalData(CanonicalError::AssemblyRecipe(error))
    })
}

fn read_product(
    reader: &mut Reader<'_>,
    capabilities: ProductSchemaCapabilities,
    point_profiles: &mut BTreeSet<FeatureId>,
) -> Result<ProductModel, PersistenceError> {
    let mut product = ProductModel {
        document_id: crate::document::DocumentId(reader.u64()?),
        units: match reader.u8()? {
            1 => UnitSystem::Millimetres,
            units => {
                return Err(PersistenceError::Legacy(LegacyError::UnsupportedUnits(
                    units,
                )));
            }
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
                return Err(PersistenceError::Legacy(LegacyError::DuplicateOverride));
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
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicateFeatureParameterBinding,
                    ));
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
            return Err(PersistenceError::Legacy(LegacyError::DuplicateDefinition(
                id,
            )));
        }
    }
    for _ in 0..reader.count()? {
        let id = FeatureId(reader.u64()?);
        let definition_id = DefinitionId(reader.u64()?);
        let name = reader.string()?;
        let kind = match reader.u8()? {
            17 if capabilities.workplane_sketch => FeatureKind::Workplane(read_workplane(
                reader,
                capabilities.free_workplanes,
                capabilities.construction_plane_workplanes,
            )?),
            18 if capabilities.workplane_sketch => FeatureKind::Sketch(read_sketch(
                reader,
                capabilities.sketch_constraint_vocabulary,
                capabilities.cubic_bezier_sketch,
                capabilities.sketch_projection,
                capabilities.sketch_construction,
            )?),
            1 => {
                let mut points_mm = Vec::new();
                for _ in 0..reader.count()? {
                    points_mm.push([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)]);
                }
                point_profiles.insert(id);
                FeatureKind::polygon(&points_mm)
            }
            11 if capabilities.segment_profile => {
                let closed = match reader.u8()? {
                    0 => false,
                    1 => true,
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
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
                                    return Err(PersistenceError::Legacy(
                                        LegacyError::InvalidFeatureKind(value),
                                    ));
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
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    });
                }
                FeatureKind::Profile { segments, closed }
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
                                value => {
                                    return Err(PersistenceError::Legacy(
                                        LegacyError::InvalidFeatureKind(value),
                                    ));
                                }
                            },
                        },
                        3 => SpatialPathSegment::CubicBezier {
                            start_mm: point(reader)?,
                            control_1_mm: point(reader)?,
                            control_2_mm: point(reader)?,
                            end_mm: point(reader)?,
                        },
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    });
                }
                FeatureKind::SpatialPath { segments }
            }
            26 if capabilities.construction_geometry => FeatureKind::ConstructionPoint {
                position_mm: [
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                ],
            },
            27 if capabilities.construction_axis => FeatureKind::ConstructionAxis {
                origin_mm: [
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                ],
                direction: [
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                ],
            },
            28 if capabilities.construction_plane => FeatureKind::ConstructionPlane {
                origin_mm: [
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                ],
                normal: [
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                ],
                x_direction: [
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                    f64::from_bits(reader.u64()?),
                ],
            },
            14 if capabilities.loft_spline => {
                let mut control_points_mm = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    control_points_mm
                        .push([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)]);
                }
                FeatureKind::closed_spline(&control_points_mm)
            }
            2 => FeatureKind::extrusion(
                FeatureId(reader.u64()?),
                Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            ),
            19 if capabilities.workplane_sketch => {
                let profile = read_sketch_region_profile(reader)?;
                let (direction, extent) = read_sketch_feature_extent(reader, capabilities)?;
                FeatureKind::Pad(PadSpec {
                    profile,
                    direction,
                    extent,
                    operation: PadOperation::NewBody,
                })
            }
            20 if capabilities.workplane_sketch => {
                let target = FeatureId(reader.u64()?);
                let profile = read_sketch_region_profile(reader)?;
                let (direction, extent) = read_sketch_feature_extent(reader, capabilities)?;
                FeatureKind::Pad(PadSpec {
                    profile,
                    direction,
                    extent,
                    operation: PadOperation::Cut {
                        target,
                        start: CutStart::Support(Box::new(read_exact_reference(reader)?)),
                    },
                })
            }
            3 if capabilities.through_cut => {
                let target = FeatureId(reader.u64()?);
                FeatureKind::through_cut(target, FeatureId(reader.u64()?))
            }
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
            // A shell that names its faces by role string has no topology to migrate to.
            5 if capabilities.shell => {
                return Err(PersistenceError::LegacyFeatureRequiresMigration {
                    feature_id: id,
                    kind: super::LegacyFeatureKind::RoleStringShell,
                });
            }
            21 if capabilities.topological_feature_references => {
                let target = FeatureId(reader.u64()?);
                let mut removed_faces = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    removed_faces.push(read_topological_reference(reader)?);
                }
                let thickness = Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?;
                let direction = if capabilities.shell_direction {
                    match reader.u8()? {
                        1 => ShellDirection::Inward,
                        2 => ShellDirection::Outward,
                        3 => ShellDirection::Symmetric,
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    }
                } else {
                    ShellDirection::Inward
                };
                let mut profile_faces = Vec::new();
                if capabilities.profile_shell_faces {
                    for _ in 0..reader.count_with_limit(64)? {
                        profile_faces.push(read_profile_face_reference(reader)?);
                    }
                }
                FeatureKind::Shell {
                    target,
                    removed_faces: removed_faces
                        .into_iter()
                        .map(FaceRef::from)
                        .chain(profile_faces.into_iter().map(FaceRef::Named))
                        .collect(),
                    thickness,
                    direction,
                }
            }
            22 if capabilities.topological_feature_references => {
                let target = FeatureId(reader.u64()?);
                let mut edges = Vec::new();
                for _ in 0..reader.count_with_limit(64)? {
                    edges.push(read_topological_reference(reader)?);
                }
                let kind = match reader.u8()? {
                    1 => EdgeFinishKind::Fillet,
                    2 => EdgeFinishKind::Chamfer,
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
                };
                let amount = Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?;
                let mut fillet_radius_stations = Vec::new();
                if capabilities.variable_fillet {
                    for _ in 0..reader.count_with_limit(32)? {
                        fillet_radius_stations.push(FilletRadiusStation {
                            position: f64::from_bits(reader.u64()?),
                            radius: Dimension::new(
                                reader.string()?,
                                f64::from_bits(reader.u64()?),
                            )?,
                        });
                    }
                }
                let (chamfer_mode, chamfer_edge_sides) = if capabilities.advanced_chamfer {
                    let mode = match reader.u8()? {
                        1 => ChamferMode::Symmetric,
                        2 => ChamferMode::TwoDistance {
                            second_distance: Dimension::new(
                                reader.string()?,
                                f64::from_bits(reader.u64()?),
                            )?,
                        },
                        3 => ChamferMode::DistanceAngle {
                            angle_degrees: f64::from_bits(reader.u64()?),
                        },
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    };
                    let mut selections = Vec::new();
                    for _ in 0..reader.count_with_limit(64)? {
                        selections.push(ChamferEdgeSide {
                            edge: read_topological_reference(reader)?,
                            side_face: read_topological_reference(reader)?,
                        });
                    }
                    (mode, selections)
                } else {
                    (ChamferMode::Symmetric, Vec::new())
                };
                let mut profile_edges = Vec::new();
                if capabilities.profile_edge_references {
                    for _ in 0..reader.count_with_limit(64)? {
                        profile_edges.push(ProfileEdgeReference {
                            first: read_profile_face_reference(reader)?,
                            second: read_profile_face_reference(reader)?,
                        });
                    }
                }
                FeatureKind::EdgeFinish {
                    target,
                    edges: edges
                        .into_iter()
                        .map(EdgeRef::from)
                        .chain(profile_edges.into_iter().map(EdgeRef::Named))
                        .collect(),
                    kind,
                    amount,
                    fillet_radius_stations,
                    chamfer_mode,
                    chamfer_edge_sides,
                }
            }
            23 if capabilities.topological_feature_references => {
                let target = FeatureId(reader.u64()?);
                let face = if capabilities.profile_face_references {
                    match reader.u8()? {
                        1 => FaceRef::from(read_topological_reference(reader)?),
                        2 => FaceRef::Named(read_profile_face_reference(reader)?),
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    }
                } else {
                    FaceRef::from(read_topological_reference(reader)?)
                };
                FeatureKind::FaceOffset {
                    target,
                    face,
                    distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                }
            }
            8 if capabilities.boolean => FeatureKind::Boolean {
                operation: match reader.u8()? {
                    1 => BooleanOperation::Cut,
                    2 => BooleanOperation::Union,
                    3 if capabilities.boolean_intersect => BooleanOperation::Intersect,
                    4 if capabilities.boolean_split => BooleanOperation::Split,
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
                },
                target: FeatureId(reader.u64()?),
                tool: FeatureId(reader.u64()?),
            },
            10 if capabilities.pocket => {
                let target = FeatureId(reader.u64()?);
                let profile = FeatureId(reader.u64()?);
                FeatureKind::pocket(
                    target,
                    profile,
                    Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                )
            }
            12 if capabilities.planar_offset => FeatureKind::PlanarOffset {
                profile: FeatureId(reader.u64()?),
                distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            13 if capabilities.sweep => FeatureKind::Sweep {
                profile: FeatureId(reader.u64()?),
                path: FeatureId(reader.u64()?),
            },
            30 if capabilities.weldment_member => FeatureKind::WeldmentMember(WeldmentMemberSpec {
                profile: FeatureId(reader.u64()?),
                path: FeatureId(reader.u64()?),
                orientation_degrees: f64::from_bits(reader.u64()?),
            }),
            31 if capabilities.weldment_joint => FeatureKind::WeldmentJoint(WeldmentJointSpec {
                first_member: FeatureId(reader.u64()?),
                second_member: FeatureId(reader.u64()?),
                policy: match reader.u8()? {
                    1 => WeldmentJointPolicy::Butt,
                    2 => WeldmentJointPolicy::Miter,
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
                },
                primary: match reader.u8()? {
                    1 => WeldmentJointPrimary::First,
                    2 => WeldmentJointPrimary::Second,
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
                },
            }),
            32 if capabilities.surface_body => FeatureKind::SurfaceBody(match reader.u8()? {
                1 => SurfaceBodySpec::Planar {
                    profile: FeatureId(reader.u64()?),
                },
                2 => {
                    let mut sections = Vec::new();
                    for _ in 0..reader.count_with_limit(16)? {
                        sections.push(LoftSection {
                            profile: FeatureId(reader.u64()?),
                            elevation_mm: f64::from_bits(reader.u64()?),
                        });
                    }
                    let guide = match reader.u8()? {
                        0 => None,
                        1 => Some(FeatureId(reader.u64()?)),
                        value => return Err(PersistenceError::InvalidBoolean(value)),
                    };
                    let continuity = match reader.u8()? {
                        1 => LoftContinuity::Position,
                        2 => LoftContinuity::Tangent,
                        3 => LoftContinuity::Curvature,
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    };
                    SurfaceBodySpec::Loft {
                        sections,
                        guide,
                        continuity,
                    }
                }
                value => {
                    return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                        value,
                    )));
                }
            }),
            33 if capabilities.surface_operations => FeatureKind::SurfaceTrim {
                target: FeatureId(reader.u64()?),
                cutter: FeatureId(reader.u64()?),
            },
            34 if capabilities.surface_operations => FeatureKind::SurfaceExtend {
                target: FeatureId(reader.u64()?),
                distance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            35 if capabilities.surface_knit => {
                let mut surfaces = Vec::new();
                for _ in 0..reader.count_with_limit(256)? {
                    surfaces.push(FeatureId(reader.u64()?));
                }
                FeatureKind::SurfaceKnit {
                    surfaces,
                    tolerance: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                    make_solid: match reader.u8()? {
                        0 => false,
                        1 => true,
                        value => return Err(PersistenceError::InvalidBoolean(value)),
                    },
                }
            }
            36 if capabilities.surface_thicken => FeatureKind::SurfaceThicken {
                target: FeatureId(reader.u64()?),
                thickness: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                direction: match reader.u8()? {
                    1 => ShellDirection::Inward,
                    2 => ShellDirection::Outward,
                    3 => ShellDirection::Symmetric,
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
                },
            },
            15 if capabilities.loft_spline => {
                let mut sections = Vec::new();
                for _ in 0..reader.count_with_limit(16)? {
                    sections.push(LoftSection {
                        profile: FeatureId(reader.u64()?),
                        elevation_mm: f64::from_bits(reader.u64()?),
                    });
                }
                let (guide, continuity) = if capabilities.loft_guide_continuity {
                    let guide = match reader.u8()? {
                        0 => None,
                        1 => Some(FeatureId(reader.u64()?)),
                        value => return Err(PersistenceError::InvalidBoolean(value)),
                    };
                    let continuity = match reader.u8()? {
                        1 => LoftContinuity::Position,
                        2 => LoftContinuity::Tangent,
                        3 => LoftContinuity::Curvature,
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    };
                    (guide, continuity)
                } else {
                    (None, LoftContinuity::Position)
                };
                FeatureKind::Loft {
                    sections,
                    guide,
                    continuity,
                }
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
            29 if capabilities.sheet_metal => {
                let width = Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?;
                let depth = Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?;
                let thickness = Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?;
                let k_factor = f64::from_bits(reader.u64()?);
                let mut flanges = Vec::new();
                for _ in 0..reader.count_with_limit(4)? {
                    flanges.push(SheetMetalFlange {
                        edge: match reader.u8()? {
                            1 => SheetMetalEdge::MinX,
                            2 => SheetMetalEdge::MaxX,
                            3 => SheetMetalEdge::MinY,
                            4 => SheetMetalEdge::MaxY,
                            value => {
                                return Err(PersistenceError::Legacy(
                                    LegacyError::InvalidFeatureKind(value),
                                ));
                            }
                        },
                        length: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
                        angle_degrees: f64::from_bits(reader.u64()?),
                        inner_radius: Dimension::new(
                            reader.string()?,
                            f64::from_bits(reader.u64()?),
                        )?,
                    });
                }
                FeatureKind::SheetMetal(SheetMetalSpec {
                    width,
                    depth,
                    thickness,
                    k_factor,
                    flanges,
                })
            }
            16 if capabilities.imported_exact_body => {
                let schema = reader.string()?;
                let import_id = ImportId(reader.u64()?);
                let source_sha256 = reader
                    .take(32)?
                    .try_into()
                    .map_err(|_| PersistenceError::Truncated)?;
                let source_byte_len = reader.u64()?;
                let source_part_index = if capabilities.imported_exact_part {
                    match reader.u8()? {
                        0 => None,
                        1 => Some(reader.u32()?),
                        value => return Err(PersistenceError::InvalidBoolean(value)),
                    }
                } else {
                    None
                };
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
                let (body_kind, area_mm2) = if capabilities.imported_exact_body_kind {
                    let body_kind = match reader.u8()? {
                        1 => BodyKind::Solid,
                        2 => BodyKind::Surface,
                        value => {
                            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                                value,
                            )));
                        }
                    };
                    (body_kind, f64::from_bits(reader.u64()?))
                } else {
                    (BodyKind::Solid, 0.0)
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
                    source_part_index,
                    result_fingerprint,
                    body_kind,
                    solid_count,
                    topology_counts,
                    area_mm2,
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
                    5 if capabilities.glb_import => MeshAuthority::ImportedGlb {
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
                            value => {
                                return Err(PersistenceError::Legacy(
                                    LegacyError::InvalidFeatureKind(value),
                                ));
                            }
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
                    value => {
                        return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                            value,
                        )));
                    }
                };
                FeatureKind::MeshBody(MeshBodySpec {
                    schema,
                    vertices_mm,
                    triangles,
                    authority,
                })
            }
            kind => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                    kind,
                )));
            }
        };
        let feature = Feature {
            id,
            definition_id,
            name,
            kind,
        };
        if product.features.insert(id, Arc::new(feature)).is_some() {
            return Err(PersistenceError::Legacy(LegacyError::DuplicateFeature(id)));
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
            color: if capabilities.occurrence_colors && reader.boolean()? {
                Some([reader.u8()?, reader.u8()?, reader.u8()?])
            } else {
                None
            },
        };
        if product
            .occurrences
            .insert(id, Arc::new(occurrence))
            .is_some()
        {
            return Err(PersistenceError::Legacy(LegacyError::DuplicateOccurrence(
                id,
            )));
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
            return Err(PersistenceError::Legacy(LegacyError::DuplicateGroup(id)));
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
                return Err(PersistenceError::Legacy(LegacyError::DuplicateLocalGroup(
                    key,
                )));
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
                color: if capabilities.occurrence_colors && reader.boolean()? {
                    Some([reader.u8()?, reader.u8()?, reader.u8()?])
                } else {
                    None
                },
            };
            if product
                .local_occurrences
                .insert(key, Arc::new(occurrence))
                .is_some()
            {
                return Err(PersistenceError::Legacy(
                    LegacyError::DuplicateLocalOccurrence(key),
                ));
            }
        }
        if !reader.is_finished() {
            for _ in 0..reader.count()? {
                let joint = read_joint(reader)?;
                if product.joints.insert(joint.id(), Arc::new(joint)).is_some() {
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateJoint));
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
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicateExactReference,
                    ));
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
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicatePersistentDimension(dimension.id),
                    ));
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
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateTag(id)));
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
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateCollection(
                        id,
                    )));
                }
            }
        }
        if capabilities.import_receipts {
            for _ in 0..reader.count_with_limit(MAX_IMPORT_OUTPUTS as u32)? {
                let receipt = read_import_receipt(
                    reader,
                    capabilities.sketchup_scene,
                    capabilities.glb_import,
                    capabilities.iges_import,
                )?;
                let id = receipt.id();
                if product
                    .import_receipts
                    .insert(id, Arc::new(receipt))
                    .is_some()
                {
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateImport(id)));
                }
            }
        }
        if capabilities.space_clearance {
            for _ in 0..reader.count()? {
                let space = read_space(reader)?;
                if product.spaces.insert(space.id(), Arc::new(space)).is_some() {
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateSpace));
                }
            }
            for _ in 0..reader.count()? {
                let clearance = read_clearance_volume(reader)?;
                if product
                    .clearance_volumes
                    .insert(clearance.id(), Arc::new(clearance))
                    .is_some()
                {
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicateClearanceVolume,
                    ));
                }
            }
        }
        if capabilities.assembly_contract {
            for _ in 0..reader.count()? {
                let id = OccurrenceId(reader.u64()?);
                if !product.grounded_occurrences.insert(id) {
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicateGroundedOccurrence(id),
                    ));
                }
            }
            for _ in 0..reader.count()? {
                let mate = read_assembly_mate(
                    reader,
                    capabilities.planar_face_attachments,
                    capabilities.axial_attachments,
                    capabilities.nested_assembly_paths,
                )?;
                if product
                    .assembly_mates
                    .insert(mate.id(), Arc::new(mate))
                    .is_some()
                {
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateAssemblyMate));
                }
            }
        }
        if capabilities.orthographic_drawing && !reader.is_finished() {
            for _ in 0..reader.count()? {
                let sheet = read_drawing_sheet_with_annotations(
                    reader,
                    capabilities.drawing_page_contract,
                    capabilities.drawing_view_contract,
                    capabilities.drawing_section_contract,
                    capabilities.drawing_detail_contract,
                    capabilities.drawing_dimension_contract,
                    (
                        capabilities.drawing_tolerance_contract,
                        capabilities.drawing_annotation_contract,
                    ),
                    capabilities.nested_drawing_paths,
                    capabilities.typed_drawing_dimensions,
                    capabilities.drawing_gdt,
                    capabilities.drawing_bom_balloons,
                )?;
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
                    return Err(PersistenceError::Legacy(LegacyError::DuplicateDefinition(
                        definition_id,
                    )));
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
                // Older files stored planar offsets as features without a body;
                // they now produce a surface body on the active body.
                for (feature_id, ownership) in &mut feature_body_ownership {
                    if ownership.output_body_id().is_none()
                        && matches!(
                            product
                                .features
                                .get(feature_id)
                                .map(|feature| &feature.kind),
                            Some(FeatureKind::PlanarOffset { .. })
                        )
                    {
                        *ownership = FeatureBodyOwnership::new(
                            ownership.input_body_ids().to_vec(),
                            Some(active_body_id),
                        )?;
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
                let joint = read_assembly_joint(
                    reader,
                    capabilities.helical_assembly_joints,
                    capabilities.nested_assembly_paths,
                )?;
                if product
                    .assembly_joints
                    .insert(joint.id(), Arc::new(joint))
                    .is_some()
                {
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicateAssemblyJoint,
                    ));
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
                        return Err(PersistenceError::Legacy(
                            LegacyError::DuplicateAssemblyMotionCoupling,
                        ));
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
                    return Err(PersistenceError::Legacy(
                        LegacyError::DuplicateAssemblyMotionStudy,
                    ));
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
                        return Err(PersistenceError::Legacy(
                            LegacyError::DuplicateMechanicalInterface,
                        ));
                    }
                }
                for _ in 0..reader.count()? {
                    let condition = read_mechanical_condition(reader)?;
                    if product
                        .mechanical_conditions
                        .insert(condition.id(), Arc::new(condition))
                        .is_some()
                    {
                        return Err(PersistenceError::Legacy(
                            LegacyError::DuplicateMechanicalCondition,
                        ));
                    }
                }
            }
        }
        if capabilities.cam_plans && !reader.is_finished() {
            for _ in 0..reader.count_with_limit(MAX_COLLECTION_ITEMS)? {
                let plan = read_cam_plan(reader)?;
                if product
                    .cam_plans
                    .insert(plan.id(), Arc::new(plan))
                    .is_some()
                {
                    return Err(PersistenceError::InvalidCanonicalData(CanonicalError::Cam(
                        crate::cam::CamError::InvalidPlan,
                    )));
                }
            }
        }
        if capabilities.nested_instance_transforms && !reader.is_finished() {
            for _ in 0..reader.count_with_limit(MAX_COLLECTION_ITEMS)? {
                let path = read_instance_path(reader)?;
                let transform = reader.transform()?;
                if product
                    .instance_transform_overrides
                    .insert(path, transform)
                    .is_some()
                {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::InvalidInstancePath,
                    ));
                }
            }
        }
        if capabilities.dowel_joinery && !reader.is_finished() {
            for _ in 0..reader.count_with_limit(MAX_COLLECTION_ITEMS)? {
                let joint = read_dowel_joint(
                    reader,
                    capabilities.dowel_physical_hole_bindings,
                    capabilities.dowel_pair_offsets,
                )?;
                if product
                    .dowel_joints
                    .insert(joint.id, Arc::new(joint))
                    .is_some()
                {
                    return Err(PersistenceError::InvalidCanonicalData(
                        CanonicalError::DowelJoint(crate::joinery::DowelJointError::InvalidJointId),
                    ));
                }
            }
        }
    }
    if capabilities.production_codes && !reader.is_finished() {
        let mut codes = BTreeSet::new();
        for _ in 0..reader.count_with_limit(MAX_COLLECTION_ITEMS)? {
            let path = read_instance_path(reader)?;
            let length = reader.count_with_limit(64)? as usize;
            let code = std::str::from_utf8(reader.take(length)?)
                .map_err(|_| PersistenceError::InvalidUtf8)?
                .to_owned();
            crate::document::validate_production_code(&code)?;
            if !codes.insert(code.to_ascii_uppercase()) {
                return Err(PersistenceError::InvalidCanonicalData(
                    CanonicalError::DuplicateProductionCode(code),
                ));
            }
            if product
                .production_codes
                .insert(path.clone(), code)
                .is_some()
            {
                return Err(PersistenceError::InvalidCanonicalData(
                    CanonicalError::InvalidProductionCodePath(path),
                ));
            }
        }
    }
    if capabilities.assembly_recipe && !reader.is_finished() {
        product.assembly_recipe = match reader.u8()? {
            1 => Some(Arc::new(read_assembly_recipe(reader)?)),
            value => {
                return Err(PersistenceError::Legacy(LegacyError::InvalidRecipeValue(
                    value,
                )));
            }
        };
    }
    if !capabilities.body_contract {
        crate::document::migrate_legacy_body_contract(&mut product)?;
    }
    Ok(product)
}

fn invalid_drawing_sheet() -> PersistenceError {
    PersistenceError::InvalidCanonicalData(CanonicalError::Drawing(
        crate::drawing::DrawingError::InvalidSheet,
    ))
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

/// Rejections that only the frozen field-by-field reader can produce.
#[derive(Debug, PartialEq, Eq)]
pub enum LegacyError {
    InvalidEnvelopeLength,
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
    InvalidRecipeValue(u8),
    InvalidPersistentDimensionTarget(u8),
    InvalidDimensionDisplayUnit(u8),
    InvalidClearanceOwner(u8),
    InvalidClearanceCoordinateFrame,
    InvalidClearanceSeverity(u8),
    InvalidAssemblyMate,
    InvalidAssemblyJoint,
    InvalidAssemblyMotionCoupling,
    InvalidAssemblyMotionStudy,
    InvalidMechanicalInterface,
    InvalidMechanicalCondition,
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
}

impl fmt::Display for LegacyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvelopeLength => {
                formatter.write_str("document envelope length is invalid")
            }
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
            Self::InvalidRecipeValue(value) => {
                write!(formatter, "assembly recipe enum value {value} is invalid")
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
        }
    }
}
