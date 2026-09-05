use crate::append_feature::plan_feature_kind;
use crate::creation::plan_creation;
use crate::diagnostics::{
    AssistantPlanningResult, assistant_canonical_rejection, assistant_planning_rejection,
    assistant_rejection,
};
use crate::transforms::{
    rotation_in_parent_space, translated_transform, world_axis_rotation_transform,
    world_plane_mirror_transform,
};
use ketchup_core::assistant_sidecar::{
    AssistantCadBodyFeature, AssistantCadDeletePolicy, AssistantCadEditOperation,
    AssistantCadEditProgram, AssistantCadEntitySelector, AssistantRejectionDiagnostic,
    AssistantRejectionPhase,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, OccurrenceId, Snapshot, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{ExactPlanarOffsetRequest, ExactResultRegistry};
use ketchup_core::sketch::SketchConstraintId;
use ketchup_interaction::Vec3;
use std::collections::{BTreeMap, BTreeSet};

fn resolve_assistant_cad_selector(
    current_selection: &BTreeSet<OccurrenceId>,
    snapshot: &Snapshot,
    selector: &AssistantCadEntitySelector,
    operation: &str,
) -> AssistantPlanningResult<Vec<OccurrenceId>> {
    let ids = match selector {
        AssistantCadEntitySelector::CurrentSelection {} => {
            current_selection.iter().copied().collect::<Vec<_>>()
        }
        AssistantCadEntitySelector::Occurrences { occurrence_ids } => occurrence_ids
            .iter()
            .copied()
            .map(OccurrenceId)
            .collect::<Vec<_>>(),
    };
    selector
        .validate_resolved_target_count(ids.len())
        .map_err(|error| {
            assistant_planning_rejection(
                "planning.cad_selector_invalid",
                operation,
                "occurrence_selection",
                error,
                "Select between one and 100 root occurrences that still exist, then retry.",
            )
        })?;
    if let Some(id) = ids.iter().find(|id| snapshot.occurrence(**id).is_none()) {
        return Err(assistant_canonical_rejection(
            CanonicalError::OccurrenceNotFound(*id),
            operation,
            &format!("occurrence:{}", id.0),
        ));
    }
    Ok(ids)
}

/// Plans against the current document and explicit host-provided context without mutation.
///
/// An empty selection is valid unless an operation uses `CurrentSelection`.
/// Explicit selectors never fall back to selection. Topology references must be
/// current in the supplied registry. The returned batch still requires canonical
/// preview/proposal validation before commit; this function retains the existing
/// appended-feature exact graph and planar-offset preflight gates, not worker execution.
pub fn plan_assistant_cad_edit_program(
    document: &DocumentStore,
    current_selection: &BTreeSet<OccurrenceId>,
    topology_results: &ExactResultRegistry,
    program: &AssistantCadEditProgram,
) -> Result<CommandBatch, Box<AssistantRejectionDiagnostic>> {
    let snapshot = document.current();
    let document_target = format!("document:{}", snapshot.document_id().0);
    program.validate().map_err(|error| {
        assistant_rejection(
            AssistantRejectionPhase::IntentValidation,
            "intent.cad_edit_program_invalid",
            "cad_edit_program",
            &document_target,
            error,
            "Return a bounded CAD edit program that satisfies the Assistant schema invariants.",
            true,
        )
    })?;

    let mut commands = Vec::new();
    let mut appended_exact_features = Vec::new();
    let mut appended_planar_offsets = Vec::new();
    let mut working_transforms = snapshot
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.transform()))
        .collect::<BTreeMap<_, _>>();
    let mut collection_members = snapshot
        .collections()
        .map(|collection| {
            (
                collection.id(),
                collection.occurrence_ids().collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut mate_endpoints = snapshot
        .assembly_mates()
        .map(|mate| {
            (
                mate.id(),
                (
                    mate.endpoint_a().occurrence_id(),
                    mate.endpoint_b().occurrence_id(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut next_definition = snapshot
        .definitions()
        .map(|definition| definition.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_occurrence = snapshot
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let mut next_feature = snapshot
        .features()
        .map(|feature| feature.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1);

    for operation in &program.operations {
        let operation_name = match operation {
            AssistantCadEditOperation::CreateSketch { .. } => "create_sketch",
            AssistantCadEditOperation::CreatePart { .. } => "create_part",
            AssistantCadEditOperation::AppendFeature { .. } => "append_feature",
            AssistantCadEditOperation::SetDimension { .. } => "set_dimension",
            AssistantCadEditOperation::Delete { .. } => "delete_occurrence",
            AssistantCadEditOperation::Transform { .. } => "transform_occurrence",
            AssistantCadEditOperation::Copy { .. } => "copy_occurrence",
            AssistantCadEditOperation::LinearPattern { .. } => "linear_pattern_occurrence",
            AssistantCadEditOperation::Mirror { .. } => "mirror_occurrence",
        };
        let selector = match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. }
            | AssistantCadEditOperation::AppendFeature { .. }
            | AssistantCadEditOperation::SetDimension { .. } => None,
            AssistantCadEditOperation::Delete { selector, .. }
            | AssistantCadEditOperation::Transform { selector, .. }
            | AssistantCadEditOperation::Copy { selector, .. }
            | AssistantCadEditOperation::LinearPattern { selector, .. }
            | AssistantCadEditOperation::Mirror { selector, .. } => Some(selector),
        };
        let targets = selector.map_or_else(
            || Ok(Vec::new()),
            |selector| {
                resolve_assistant_cad_selector(
                    current_selection,
                    &snapshot,
                    selector,
                    operation_name,
                )
            },
        )?;
        if let Some(id) = targets
            .iter()
            .find(|id| !working_transforms.contains_key(id))
        {
            return Err(assistant_planning_rejection(
                "planning.cad_target_deleted",
                operation_name,
                &format!("occurrence:{}", id.0),
                "An earlier operation in this CAD edit program already deleted the target.",
                "Remove the later operation or target an occurrence that remains in the program.",
            ));
        }

        match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. } => {
                commands.extend(plan_creation(
                    &snapshot,
                    operation,
                    &mut next_definition,
                    &mut next_feature,
                    &mut next_occurrence,
                    &document_target,
                )?);
            }
            AssistantCadEditOperation::AppendFeature {
                definition_id,
                name,
                feature,
            } => {
                let definition_id = DefinitionId(*definition_id);
                if snapshot.definition(definition_id).is_none() {
                    return Err(assistant_canonical_rejection(
                        CanonicalError::DefinitionNotFound(definition_id),
                        operation_name,
                        &format!("definition:{}", definition_id.0),
                    ));
                }
                let kind = plan_feature_kind(
                    &snapshot,
                    topology_results,
                    definition_id,
                    feature,
                    operation_name,
                )?;
                let id = next_feature.map(FeatureId).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::IdExhausted,
                        operation_name,
                        &document_target,
                    )
                })?;
                next_feature = id.0.checked_add(1);
                commands.push(CanonicalCommand::CreateFeature {
                    id,
                    definition_id,
                    name: name.clone(),
                    kind,
                });
                if matches!(feature, AssistantCadBodyFeature::PlanarOffset { .. }) {
                    appended_planar_offsets.push((definition_id, id));
                } else {
                    appended_exact_features.push((definition_id, id));
                }
            }
            AssistantCadEditOperation::SetDimension {
                feature_id,
                constraint_id,
                value_mm,
            } => {
                let feature_id = FeatureId(*feature_id);
                let dimension =
                    Dimension::new(value_mm.to_string(), *value_mm).map_err(|error| {
                        assistant_canonical_rejection(
                            error,
                            operation_name,
                            &format!("feature:{}", feature_id.0),
                        )
                    })?;
                commands.push(if let Some(constraint_id) = constraint_id {
                    CanonicalCommand::SetSketchConstraintDimension {
                        id: feature_id,
                        constraint_id: SketchConstraintId(*constraint_id),
                        dimension,
                    }
                } else {
                    CanonicalCommand::SetFeatureDimension {
                        id: feature_id,
                        dimension,
                    }
                });
            }
            AssistantCadEditOperation::Delete {
                dependency_policy, ..
            } => {
                let target_set = targets.iter().copied().collect::<BTreeSet<_>>();
                let referenced_collections = collection_members
                    .iter()
                    .filter(|(_, members)| members.iter().any(|id| target_set.contains(id)))
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                let incident_mates = mate_endpoints
                    .iter()
                    .filter(|(_, (left, right))| {
                        target_set.contains(left) || target_set.contains(right)
                    })
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                if *dependency_policy == AssistantCadDeletePolicy::RejectIfReferenced
                    && (!referenced_collections.is_empty() || !incident_mates.is_empty())
                {
                    let id = targets[0];
                    return Err(assistant_planning_rejection(
                        "planning.cad_delete_referenced",
                        operation_name,
                        &format!("occurrence:{}", id.0),
                        "The occurrence is referenced by a collection or assembly mate.",
                        "Use remove_references only when removing those dependencies is intended.",
                    ));
                }
                if *dependency_policy == AssistantCadDeletePolicy::RemoveReferences {
                    for collection_id in referenced_collections {
                        let members = collection_members
                            .get_mut(&collection_id)
                            .expect("referenced collection came from the working map");
                        members.retain(|id| !target_set.contains(id));
                        commands.push(CanonicalCommand::SetCollectionOccurrences {
                            id: collection_id,
                            occurrence_ids: members.clone(),
                        });
                    }
                    for mate_id in incident_mates {
                        mate_endpoints.remove(&mate_id);
                        commands.push(CanonicalCommand::DeleteAssemblyMate { id: mate_id });
                    }
                }
                for id in targets {
                    working_transforms.remove(&id);
                    commands.push(CanonicalCommand::DeleteOccurrence { id });
                }
            }
            AssistantCadEditOperation::Transform {
                translation_mm,
                rotation,
                ..
            } => {
                let delta = Vec3::new(translation_mm[0], translation_mm[1], translation_mm[2]);
                let world_rotation = rotation
                    .as_ref()
                    .map(|rotation| {
                        world_axis_rotation_transform(
                            Vec3::new(
                                rotation.pivot_mm[0],
                                rotation.pivot_mm[1],
                                rotation.pivot_mm[2],
                            ),
                            Vec3::new(rotation.axis[0], rotation.axis[1], rotation.axis[2]),
                            rotation.angle_degrees,
                        )
                    })
                    .transpose()
                    .map_err(|_| {
                        assistant_planning_rejection(
                            "planning.cad_transform_invalid",
                            operation_name,
                            "occurrence_selection",
                            "The requested rigid transform could not be represented.",
                            "Use a finite translation and a non-zero finite rotation axis.",
                        )
                    })?;
                for id in targets {
                    let current = working_transforms[&id];
                    let translated = translated_transform(current, delta).map_err(|_| {
                        assistant_planning_rejection(
                            "planning.cad_transform_invalid",
                            operation_name,
                            &format!("occurrence:{}", id.0),
                            "The requested translation could not be represented.",
                            "Use a finite bounded translation.",
                        )
                    })?;
                    let transform = if let Some(world_rotation) = world_rotation {
                        let occurrence = snapshot
                            .occurrence(id)
                            .expect("resolved CAD selector targets a snapshot occurrence");
                        let parent_transform = occurrence
                            .parent()
                            .map_or(Some(Transform::identity()), |parent| {
                                snapshot.world_transform_for_group(parent)
                            })
                            .ok_or_else(|| {
                                assistant_planning_rejection(
                                    "planning.cad_parent_transform_unavailable",
                                    operation_name,
                                    &format!("occurrence:{}", id.0),
                                    "The occurrence parent transform could not be resolved.",
                                    "Refresh the document context and retry the transform.",
                                )
                            })?;
                        rotation_in_parent_space(
                            world_rotation,
                            parent_transform,
                            translated,
                        )
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.cad_transform_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The requested world-space rotation could not be represented in the occurrence parent.",
                                "Use a finite rigid parent and rotation.",
                            )
                        })?
                    } else {
                        translated
                    };
                    working_transforms.insert(id, transform);
                    commands.push(CanonicalCommand::SetOccurrenceTransform { id, transform });
                }
            }
            AssistantCadEditOperation::Copy { translation_mm, .. } => {
                let delta = Vec3::new(translation_mm[0], translation_mm[1], translation_mm[2]);
                for id in targets {
                    let source = snapshot
                        .occurrence(id)
                        .expect("resolved CAD selector targets a snapshot occurrence");
                    let transform =
                        translated_transform(working_transforms[&id], delta).map_err(|_| {
                            assistant_planning_rejection(
                                "planning.cad_copy_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The requested copy transform could not be represented.",
                                "Use a finite bounded translation.",
                            )
                        })?;
                    let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::IdExhausted,
                            operation_name,
                            &document_target,
                        )
                    })?;
                    next_occurrence = occurrence_id.0.checked_add(1);
                    commands.push(CanonicalCommand::CreateOccurrence {
                        id: occurrence_id,
                        definition_id: source.definition_id(),
                        name: source.name().to_owned(),
                        transform,
                        parent: source.parent(),
                        tag: source.tag(),
                        visible: source.visible(),
                    });
                }
            }
            AssistantCadEditOperation::LinearPattern {
                instances, step_mm, ..
            } => {
                let step = Vec3::new(step_mm[0], step_mm[1], step_mm[2]);
                for instance in 1..*instances {
                    let delta = step * f64::from(instance);
                    for id in &targets {
                        let source = snapshot
                            .occurrence(*id)
                            .expect("resolved CAD selector targets a snapshot occurrence");
                        let transform = translated_transform(working_transforms[id], delta)
                            .map_err(|_| {
                                assistant_planning_rejection(
                                    "planning.cad_pattern_invalid",
                                    operation_name,
                                    &format!("occurrence:{}", id.0),
                                    "The requested pattern transform could not be represented.",
                                    "Use a finite bounded pattern step and instance count.",
                                )
                            })?;
                        let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                            assistant_canonical_rejection(
                                CanonicalError::IdExhausted,
                                operation_name,
                                &document_target,
                            )
                        })?;
                        next_occurrence = occurrence_id.0.checked_add(1);
                        commands.push(CanonicalCommand::CreateOccurrence {
                            id: occurrence_id,
                            definition_id: source.definition_id(),
                            name: source.name().to_owned(),
                            transform,
                            parent: source.parent(),
                            tag: source.tag(),
                            visible: source.visible(),
                        });
                    }
                }
            }
            AssistantCadEditOperation::Mirror {
                plane_origin_mm,
                plane_normal,
                ..
            } => {
                let world_mirror = world_plane_mirror_transform(
                    Vec3::new(plane_origin_mm[0], plane_origin_mm[1], plane_origin_mm[2]),
                    Vec3::new(plane_normal[0], plane_normal[1], plane_normal[2]),
                )
                .map_err(|_| {
                    assistant_planning_rejection(
                        "planning.cad_mirror_invalid",
                        operation_name,
                        "occurrence_selection",
                        "The requested mirror plane could not be represented.",
                        "Use a finite plane origin and non-zero finite normal.",
                    )
                })?;
                for id in targets {
                    let source = snapshot
                        .occurrence(id)
                        .expect("resolved CAD selector targets a snapshot occurrence");
                    let parent_transform = source
                        .parent()
                        .map_or(Some(Transform::identity()), |parent| {
                            snapshot.world_transform_for_group(parent)
                        })
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.cad_parent_transform_unavailable",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The occurrence parent transform could not be resolved.",
                                "Refresh the document context and retry the mirror.",
                            )
                        })?;
                    let transform = rotation_in_parent_space(
                        world_mirror,
                        parent_transform,
                        working_transforms[&id],
                    )
                    .ok_or_else(|| {
                        assistant_planning_rejection(
                            "planning.cad_mirror_invalid",
                            operation_name,
                            &format!("occurrence:{}", id.0),
                            "The requested mirror could not be represented in the occurrence parent.",
                            "Use a finite invertible parent transform and mirror plane.",
                        )
                    })?;
                    let occurrence_id = next_occurrence.map(OccurrenceId).ok_or_else(|| {
                        assistant_canonical_rejection(
                            CanonicalError::IdExhausted,
                            operation_name,
                            &document_target,
                        )
                    })?;
                    next_occurrence = occurrence_id.0.checked_add(1);
                    commands.push(CanonicalCommand::CreateOccurrence {
                        id: occurrence_id,
                        definition_id: source.definition_id(),
                        name: source.name().to_owned(),
                        transform,
                        parent: source.parent(),
                        tag: source.tag(),
                        visible: source.visible(),
                    });
                }
            }
        }
    }
    let batch = CommandBatch::new(commands);
    if !appended_exact_features.is_empty() || !appended_planar_offsets.is_empty() {
        let candidate = document.preview_batch(&batch).map_err(|error| {
            assistant_canonical_rejection(error, "append_feature", &document_target)
        })?;
        for (definition_id, feature_id) in appended_exact_features {
            ExactBRepGraph::from_snapshot(&candidate, definition_id, feature_id).map_err(
                |error| {
                    assistant_planning_rejection(
                        "planning.cad_feature_result_unsupported",
                        "append_feature",
                        &format!("feature:{}", feature_id.0),
                        error.to_string(),
                        "Use operands and an operation that produce a supported exact body.",
                    )
                },
            )?;
        }
        for (definition_id, feature_id) in appended_planar_offsets {
            let request = ExactPlanarOffsetRequest::from_snapshot(&candidate, definition_id)
                .map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_feature_result_unsupported",
                        "append_feature",
                        &format!("feature:{}", feature_id.0),
                        error.to_string(),
                        "Use one rectangular profile and a signed distance that leaves a non-collapsing planar result.",
                    )
                })?;
            if request.offset_feature_id != feature_id {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_result_unsupported",
                    "append_feature",
                    &format!("feature:{}", feature_id.0),
                    "The Planar Offset result does not match the host-assigned output feature.",
                    "Use the sole rectangular profile in the requested definition.",
                ));
            }
        }
    }
    Ok(batch)
}
