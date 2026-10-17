use ketchup_geometry::sketch::{FeatureExtent, PadOperation, PadProfile, PadSpec};
use std::collections::BTreeSet;

use ketchup_application::model_query::{EntityKind, ModelQuery, PageRequest};
use ketchup_application::{
    AssistantCadResolvedProgramOutput, plan_assistant_cad_edit_program as plan,
    plan_assistant_cad_edit_program_with_outputs as plan_with_outputs,
};
use ketchup_assistant::sidecar::*;
use ketchup_geometry::sketch::{
    PrincipalPlane, SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
};
use ketchup_model::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind, PlanarFaceAttachment,
};
use ketchup_model::document::{
    BodyKind, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, GroupId, InstancePath, OccurrenceId, Snapshot,
    SpatialPathSegment, SurfaceBodySpec, Transform,
};
use ketchup_model::drawing::{DrawingSource, project_orthographic_drawing};
use ketchup_model::drawing_export::export_drawing;
use ketchup_model::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V14, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
};
use ketchup_model::exact_product::{ExactBodyPackage, ExactFaceRole, ExactResultRegistry};
use ketchup_model::persistence::{ContainerData, LoadOutcome, load, save, save_document_store};
use ketchup_model::testing::box_package;
use std::sync::Arc;

fn part() -> AssistantCadEditOperation {
    AssistantCadEditOperation::CreatePart {
        holes: Vec::new(),
        pockets: Vec::new(),
        name: "Editable part".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 12.0,
        }],
        constraints: vec![AssistantSketchConstraint::Radius {
            id: 1,
            entity_id: 1,
            value_mm: 12.0,
        }],
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
        translation_mm: [5.0, 6.0, 7.0],
        rotation: None,
    }
}

fn program(operations: Vec<AssistantCadEditOperation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

/// Lines from each point to the next and from the last back to the first.
fn line_loop(points: &[[f64; 2]]) -> Vec<AssistantSketchEntity> {
    (0..points.len())
        .map(|index| AssistantSketchEntity::Line {
            id: index as u64 + 1,
            start_mm: points[index],
            end_mm: points[(index + 1) % points.len()],
        })
        .collect()
}

fn explicit(id: u64) -> AssistantCadEntitySelector {
    AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![id],
    }
}

fn translate(selector: AssistantCadEntitySelector, delta: [f64; 3]) -> AssistantCadEditOperation {
    AssistantCadEditOperation::Transform {
        selector,
        translation_mm: delta,
        rotation: None,
    }
}

fn seeded() -> DocumentStore {
    let mut document = DocumentStore::new();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![part()]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    document
}

fn cam_setup_operation(definition_id: u64, feature_id: u64) -> AssistantCadEditOperation {
    AssistantCadEditOperation::UpsertCamPlan {
        plan_id: 1,
        name: "Reviewed top setup".into(),
        target_definition_id: definition_id,
        target_feature_id: feature_id,
        stock_minimum_mm: [-15.0, -15.0, -1.0],
        stock_maximum_mm: [15.0, 15.0, 31.0],
        tool_number: 1,
        tool_kind: AssistantCamToolKind::FlatEndMill,
        tool_diameter_mm: 6.0,
        flute_length_mm: 18.0,
        overall_length_mm: 50.0,
        holder_diameter_mm: 20.0,
        holder_length_mm: 35.0,
        spindle_rpm: 12_000,
        feed_mm_per_min: 900.0,
        plunge_mm_per_min: 250.0,
        work_offset: AssistantCamWorkOffset::G54,
        origin_mm: [0.0, 0.0, 31.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        safe_height_mm: 5.0,
        maximum_stepdown_mm: 2.0,
        stepover_ratio: 0.45,
        radial_allowance_mm: 0.2,
        axial_allowance_mm: 0.1,
    }
}

fn assistant_instance_path(snapshot: &Snapshot, path: &InstancePath) -> AssistantInstancePath {
    ketchup_application::model_query::assistant_instance_path(snapshot, path).unwrap()
}

fn exact_box_package(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    feature_id: FeatureId,
) -> Arc<ExactBodyPackage> {
    Arc::new(
        box_package(
            snapshot,
            definition_id,
            feature_id,
            "public-nested-assembly-result",
            &[
                ExactFaceRole::Top,
                ExactFaceRole::Bottom,
                ExactFaceRole::LinearSide,
            ],
        )
        .unwrap(),
    )
}

/// A `create_part` cuboid: a `[width, depth]` rectangle on XY extruded by the
/// third dimension, with the holes and pockets drilled into it.
fn cuboid_part(
    name: String,
    [width, depth, height]: [f64; 3],
    holes: Vec<AssistantPartHole>,
    pockets: Vec<AssistantPartPocket>,
    translation_mm: [f64; 3],
    rotation: Option<AssistantCadRotation>,
) -> AssistantCadEditOperation {
    let corners = [[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]];
    AssistantCadEditOperation::CreatePart {
        name,
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: (0..4)
            .map(|index| AssistantSketchEntity::Line {
                id: index as u64 + 1,
                start_mm: corners[index],
                end_mm: corners[(index + 1) % 4],
            })
            .collect(),
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion {
            distance_mm: height,
        },
        holes,
        pockets,
        translation_mm,
        rotation,
    }
}

mod joints_patterns_and_targets;
mod mirrored_drawing;
mod pin_joint_fabrication;
mod profiles_and_construction;
