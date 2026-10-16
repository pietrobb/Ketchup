use ciborium::Value;
use ketchup_geometry::sketch::{FeatureExtent, PadOperation, PadProfile, PadSpec};
use ketchup_geometry::sketch::{
    PrincipalPlane, SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity,
    SketchEntityId, SketchPointKind, SketchPointRef, SketchSpec, WorkplaneSpec,
};
use ketchup_model::document::{
    BooleanOperation, CanonicalCommand, CanonicalError, CollectionId, CommandBatch,
    ConvertedEntityId, DefinitionId, DerivedIdentity, Dimension, DimensionDisplayUnit,
    DimensionPresentation, DimensionReferenceHealth, DocumentStore, EvaluationIdentity,
    EvaluatorParameterEdit, FeatureId, FeatureKind, FeatureParameterBinding,
    FeatureParameterFreshness, FeatureParameterRecomputeScope, FeatureParameterStaleReason,
    FeatureParameterTarget, GroupId, InstancePath, InstancePathStep, LocalGroupId, LocalGroupKey,
    LocalOccurrenceId, LocalOccurrenceKey, LoftContinuity, LoftSection, MappingResolution, NodeId,
    OccurrenceId, ParameterPath, ParameterPathError, ParameterValueType, PersistentDimension,
    PersistentDimensionId, PersistentDimensionTarget, PortSpec, ProfileSegment, RuleOutput,
    RuleProgramSource, SavedCamera, SavedView, SavedViewId, SavedViewProblem, SceneQueryContext,
    SceneQueryError, SectionPlane, SlotPath, SlotSegment, Snapshot, SolidToolPlan,
    SpatialPathSegment, TagHolder, TagId, Transform, UnresolvedMappingReason, WorldEntityPath,
};
use ketchup_model::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
    ExactBRepPlanarLoop, ExactBRepPlanarSegment,
};
use ketchup_model::exact_product::producer_exact_graph;
use ketchup_model::persistence;
use ketchup_model::state_view::encode_semantic_state;
use ketchup_model::testing::{cbor_entry, rewrite_saved_snapshot, with_document_id};

const CABINET: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(10);
const EXTRUSION: FeatureId = FeatureId(11);
const FIRST: OccurrenceId = OccurrenceId(20);
const SECOND: OccurrenceId = OccurrenceId(21);
const GROUP: GroupId = GroupId(30);

fn height(value: &str) -> Dimension {
    Dimension::from_decimal(value).unwrap()
}

fn seed_product_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: CABINET,
                name: "Base Cabinet".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: CABINET,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[
                    [0.0, 0.0],
                    [600.0, 0.0],
                    [600.0, 580.0],
                    [0.0, 580.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: CABINET,
                name: "Extrusion".to_owned(),
                kind: FeatureKind::extrusion(PROFILE, height("720")),
            },
            CanonicalCommand::CreateGroup {
                id: GROUP,
                name: "Kitchen run".to_owned(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: CABINET,
                name: "Base Cabinet #1".to_owned(),
                transform: Transform::identity(),
                parent: Some(GROUP),
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: CABINET,
                name: "Base Cabinet #2".to_owned(),
                transform: Transform::from_translation(700.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn seed_separate_solid_tool_document(
    tool_x_mm: f64,
    tool_y_mm: f64,
    tool_depth_mm: f64,
) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(101),
                name: "Solid target".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(102),
                definition_id: DefinitionId(101),
                name: "Target profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(103),
                definition_id: DefinitionId(101),
                name: "Target body".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(102), height("50")),
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(201),
                name: "Solid tool".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(202),
                definition_id: DefinitionId(201),
                name: "Tool profile".to_owned(),
                kind: FeatureKind::polygon(&[
                    [0.0, 0.0],
                    [40.0, 0.0],
                    [40.0, tool_depth_mm],
                    [0.0, tool_depth_mm],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(203),
                definition_id: DefinitionId(201),
                name: "Tool body".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(202), height("50")),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(301),
                definition_id: DefinitionId(101),
                name: "Target occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(302),
                definition_id: DefinitionId(201),
                name: "Tool occurrence".to_owned(),
                transform: Transform::from_translation(tool_x_mm, tool_y_mm, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn separate_solid_tool_plan(operation: BooleanOperation, keep_tool: bool) -> SolidToolPlan {
    SolidToolPlan {
        operation,
        target_occurrence_id: OccurrenceId(301),
        target_feature_id: FeatureId(103),
        tool_occurrence_id: OccurrenceId(302),
        tool_feature_id: FeatureId(203),
        result_definition_id: DefinitionId(401),
        result_feature_ids: vec![
            FeatureId(402),
            FeatureId(403),
            FeatureId(404),
            FeatureId(405),
            FeatureId(406),
            FeatureId(407),
            FeatureId(408),
        ],
        result_definition_name: "Solid Tool result".to_owned(),
        result_feature_name: "Boolean result".to_owned(),
        keep_tool,
    }
}

fn circular_profile_segments(clockwise: bool) -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::CircularArc {
            start_mm: [10.0, 0.0],
            end_mm: [-10.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise,
        },
        ProfileSegment::CircularArc {
            start_mm: [-10.0, 0.0],
            end_mm: [10.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise,
        },
    ]
}

/// The exact graph of the definition's planar offset feature.
fn offset_graph(snapshot: &Snapshot, definition_id: DefinitionId) -> ExactBRepGraph {
    let offset = snapshot
        .definition(definition_id)
        .unwrap()
        .feature_ids()
        .iter()
        .copied()
        .find(|id| {
            matches!(
                snapshot.feature(*id).map(|feature| feature.kind()),
                Some(FeatureKind::PlanarOffset { .. })
            )
        })
        .unwrap();
    ExactBRepGraph::from_snapshot(snapshot, definition_id, offset).unwrap()
}

fn offset_distance(graph: &ExactBRepGraph) -> f64 {
    let ExactBRepOperation::PlanarOffset { distance_bits, .. } =
        graph.nodes.last().unwrap().operation
    else {
        panic!("expected a planar offset graph");
    };
    f64::from_bits(distance_bits)
}

mod solid_tools_and_profiles;
mod structure_and_parameters;
