use crate::diagnostics::{
    AssistantPlanningResult, assistant_canonical_rejection, assistant_planning_rejection,
};
use crate::sketch::{
    assistant_principal_plane, assistant_sketch_constraint, assistant_sketch_entity,
};
use crate::transforms::{
    rotation_in_parent_space, translated_transform, world_axis_rotation_transform,
};
use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantCadPartFeature, AssistantWorkplaneSpec,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, DefinitionId, Dimension, FeatureId, FeatureKind,
    OccurrenceId, Snapshot, Transform,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, SketchSpec, WorkplaneSpec, WorkplaneSupport,
};
use ketchup_interaction::Vec3;

pub(crate) fn plan_creation(
    snapshot: &Snapshot,
    operation: &AssistantCadEditOperation,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    document_target: &str,
) -> AssistantPlanningResult<Vec<CanonicalCommand>> {
    let operation_name = match operation {
        AssistantCadEditOperation::CreateSketch { .. } => "create_sketch",
        AssistantCadEditOperation::CreatePart { .. } => "create_part",
        _ => unreachable!("creation dispatch accepts only sketch and part operations"),
    };
    let mut commands = Vec::new();
    match operation {
        AssistantCadEditOperation::CreateSketch {
            definition_id,
            name,
            workplane,
            entities,
            constraints,
        } => {
            let definition_id = DefinitionId(*definition_id);
            if snapshot.definition(definition_id).is_none() {
                return Err(assistant_canonical_rejection(
                    CanonicalError::DefinitionNotFound(definition_id),
                    operation_name,
                    &format!("definition:{}", definition_id.0),
                ));
            }
            let workplane_id = next_feature.map(FeatureId).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::IdExhausted,
                    operation_name,
                    document_target,
                )
            })?;
            let sketch_id = workplane_id
                .0
                .checked_add(1)
                .map(FeatureId)
                .ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        document_target,
                    )
                })?;
            *next_feature = sketch_id.0.checked_add(1);
            let workplane = match workplane {
                AssistantWorkplaneSpec::Principal { plane } => {
                    WorkplaneSpec::principal(assistant_principal_plane(*plane))
                }
                AssistantWorkplaneSpec::Offset {
                    base_feature_id,
                    distance_mm,
                } => {
                    let base = FeatureId(*base_feature_id);
                    let base_frame = snapshot
                        .feature(base)
                        .and_then(|feature| match feature.kind() {
                            FeatureKind::Workplane(spec) => Some(spec.frame),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.workplane_base_unavailable",
                                operation_name,
                                &format!("feature:{}", base.0),
                                "The requested offset base is not an existing workplane.",
                                "Refresh document context and target an existing workplane.",
                            )
                        })?;
                    let distance =
                        Dimension::new(distance_mm.to_string(), *distance_mm).map_err(|error| {
                            assistant_canonical_rejection(
                                error,
                                operation_name,
                                &format!("feature:{}", base.0),
                            )
                        })?;
                    WorkplaneSpec {
                        support: WorkplaneSupport::Offset { base, distance },
                        frame: base_frame.offset(*distance_mm),
                    }
                }
            };
            let constraints = constraints
                .iter()
                .map(assistant_sketch_constraint)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    assistant_canonical_rejection(
                        error,
                        operation_name,
                        &format!("feature:{}", sketch_id.0),
                    )
                })?;
            commands.extend([
                CanonicalCommand::CreateFeature {
                    id: workplane_id,
                    definition_id,
                    name: format!("{name} workplane"),
                    kind: FeatureKind::Workplane(workplane),
                },
                CanonicalCommand::CreateFeature {
                    id: sketch_id,
                    definition_id,
                    name: name.clone(),
                    kind: FeatureKind::Sketch(SketchSpec {
                        workplane: workplane_id,
                        entities: entities.iter().map(assistant_sketch_entity).collect(),
                        constraints,
                    }),
                },
            ]);
        }
        AssistantCadEditOperation::CreatePart {
            name,
            workplane,
            entities,
            constraints,
            feature,
            translation_mm,
            rotation,
        } => {
            let definition_id = next_definition.map(DefinitionId).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::IdExhausted,
                    operation_name,
                    document_target,
                )
            })?;
            *next_definition = definition_id.0.checked_add(1);
            let workplane_id = next_feature.map(FeatureId).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::IdExhausted,
                    operation_name,
                    document_target,
                )
            })?;
            let sketch_id = workplane_id
                .0
                .checked_add(1)
                .map(FeatureId)
                .ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        document_target,
                    )
                })?;
            let body_id = sketch_id.0.checked_add(1).map(FeatureId).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::IdExhausted,
                    operation_name,
                    document_target,
                )
            })?;
            *next_feature = body_id.0.checked_add(1);
            let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::IdExhausted,
                    operation_name,
                    document_target,
                )
            })?;
            *next_occurrence = occurrence_id.0.checked_add(1);
            let workplane = match workplane {
                AssistantWorkplaneSpec::Principal { plane } => {
                    WorkplaneSpec::principal(assistant_principal_plane(*plane))
                }
                AssistantWorkplaneSpec::Offset {
                    base_feature_id,
                    distance_mm,
                } => {
                    let base = FeatureId(*base_feature_id);
                    let base_frame = snapshot
                        .feature(base)
                        .and_then(|feature| match feature.kind() {
                            FeatureKind::Workplane(spec) => Some(spec.frame),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.workplane_base_unavailable",
                                operation_name,
                                &format!("feature:{}", base.0),
                                "The requested offset base is not an existing workplane.",
                                "Refresh document context and target an existing workplane.",
                            )
                        })?;
                    let distance =
                        Dimension::new(distance_mm.to_string(), *distance_mm).map_err(|error| {
                            assistant_canonical_rejection(
                                error,
                                operation_name,
                                &format!("feature:{}", base.0),
                            )
                        })?;
                    WorkplaneSpec {
                        support: WorkplaneSupport::Offset { base, distance },
                        frame: base_frame.offset(*distance_mm),
                    }
                }
            };
            let constraints = constraints
                .iter()
                .map(assistant_sketch_constraint)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    assistant_canonical_rejection(
                        error,
                        operation_name,
                        &format!("feature:{}", sketch_id.0),
                    )
                })?;
            let sketch = SketchSpec {
                workplane: workplane_id,
                entities: entities.iter().map(assistant_sketch_entity).collect(),
                constraints,
            };
            let regions = sketch.solved_regions().map_err(|error| {
                assistant_canonical_rejection(
                    CanonicalError::Sketch(error),
                    operation_name,
                    &format!("feature:{}", sketch_id.0),
                )
            })?;
            let [region] = regions.as_slice() else {
                return Err(assistant_planning_rejection(
                    "planning.cad_part_region_ambiguous",
                    operation_name,
                    &format!("feature:{}", sketch_id.0),
                    "The part sketch must resolve to exactly one closed region.",
                    "Return one closed, fully constrained sketch region for this part.",
                ));
            };
            let body_kind = match feature {
                AssistantCadPartFeature::Extrusion { distance_mm } => {
                    let height =
                        Dimension::new(distance_mm.to_string(), *distance_mm).map_err(|error| {
                            assistant_canonical_rejection(
                                error,
                                operation_name,
                                &format!("feature:{}", body_id.0),
                            )
                        })?;
                    FeatureKind::Pad(PadSpec {
                        sketch: sketch_id,
                        region: region.id,
                        direction: FeatureDirection::AlongNormal,
                        extent: FeatureExtent::Blind(height),
                    })
                }
                AssistantCadPartFeature::Revolve {
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                } => FeatureKind::Revolve {
                    profile: sketch_id,
                    axis_start_mm: *axis_start_mm,
                    axis_end_mm: *axis_end_mm,
                    angle_degrees: *angle_degrees,
                },
            };
            let translated = translated_transform(
                Transform::identity(),
                Vec3::new(translation_mm[0], translation_mm[1], translation_mm[2]),
            )
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_part_placement_invalid",
                    operation_name,
                    &format!("occurrence:{}", occurrence_id.0),
                    "The requested part translation could not be represented.",
                    "Use a finite bounded translation.",
                )
            })?;
            let transform = if let Some(rotation) = rotation {
                let world_rotation = world_axis_rotation_transform(
                    Vec3::new(
                        rotation.pivot_mm[0],
                        rotation.pivot_mm[1],
                        rotation.pivot_mm[2],
                    ),
                    Vec3::new(rotation.axis[0], rotation.axis[1], rotation.axis[2]),
                    rotation.angle_degrees,
                )
                .map_err(|_| {
                    assistant_planning_rejection(
                        "planning.cad_part_placement_invalid",
                        operation_name,
                        &format!("occurrence:{}", occurrence_id.0),
                        "The requested part rotation could not be represented.",
                        "Use a finite pivot and non-zero finite rotation axis.",
                    )
                })?;
                rotation_in_parent_space(world_rotation, Transform::identity(), translated)
                    .ok_or_else(|| {
                        assistant_planning_rejection(
                            "planning.cad_part_placement_invalid",
                            operation_name,
                            &format!("occurrence:{}", occurrence_id.0),
                            "The requested part placement is not a representable rigid transform.",
                            "Use a finite rigid translation and rotation.",
                        )
                    })?
            } else {
                translated
            };
            commands.extend([
                CanonicalCommand::CreateDefinition {
                    id: definition_id,
                    name: name.clone(),
                },
                CanonicalCommand::CreateFeature {
                    id: workplane_id,
                    definition_id,
                    name: format!("{name} workplane"),
                    kind: FeatureKind::Workplane(workplane),
                },
                CanonicalCommand::CreateFeature {
                    id: sketch_id,
                    definition_id,
                    name: format!("{name} sketch"),
                    kind: FeatureKind::Sketch(sketch),
                },
                CanonicalCommand::CreateFeature {
                    id: body_id,
                    definition_id,
                    name: format!("{name} feature"),
                    kind: body_kind,
                },
                CanonicalCommand::CreateOccurrence {
                    id: occurrence_id,
                    definition_id,
                    name: name.clone(),
                    transform,
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]);
        }
        _ => unreachable!("creation dispatch accepts only sketch and part operations"),
    }
    Ok(commands)
}
