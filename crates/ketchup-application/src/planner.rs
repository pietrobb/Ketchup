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
    AssistantCadEditProgram, AssistantCadEntitySelector, AssistantCadFeatureReference,
    AssistantRejectionDiagnostic, AssistantRejectionPhase,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, ClassificationCategoryId, ClassificationDimensionId,
    CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, NodeId, OccurrenceId,
    Snapshot, Transform,
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

fn resolve_program_feature_reference(
    reference: AssistantCadFeatureReference,
    original_snapshot: &Snapshot,
    operation_outputs: &BTreeMap<usize, FeatureId>,
    operation: &str,
) -> AssistantPlanningResult<u64> {
    match reference {
        AssistantCadFeatureReference::Existing(id)
            if original_snapshot.feature(FeatureId(id)).is_some() =>
        {
            Ok(id)
        }
        AssistantCadFeatureReference::Existing(id) => Err(assistant_canonical_rejection(
            CanonicalError::FeatureNotFound(FeatureId(id)),
            operation,
            &format!("feature:{id}"),
        )),
        AssistantCadFeatureReference::ProgramOutput(reference) => operation_outputs
            .get(&(reference.operation_index as usize))
            .map(|id| id.0)
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_program_feature_reference_unavailable",
                    operation,
                    &format!("operation:{}", reference.operation_index),
                    "The referenced earlier operation did not produce an available body feature.",
                    "Reference the body_feature output of an earlier create_part or append_feature operation.",
                )
            }),
    }
}

fn resolve_program_feature_references(
    feature: &AssistantCadBodyFeature,
    original_snapshot: &Snapshot,
    operation_outputs: &BTreeMap<usize, FeatureId>,
    operation: &str,
) -> AssistantPlanningResult<AssistantCadBodyFeature> {
    let mut feature = feature.clone();
    if let AssistantCadBodyFeature::Boolean {
        target_feature_id,
        tool_feature_id,
        ..
    } = &mut feature
    {
        let target = resolve_program_feature_reference(
            *target_feature_id,
            original_snapshot,
            operation_outputs,
            operation,
        )?;
        let tool = resolve_program_feature_reference(
            *tool_feature_id,
            original_snapshot,
            operation_outputs,
            operation,
        )?;
        if target == tool {
            return Err(assistant_planning_rejection(
                "planning.cad_feature_inputs_identical",
                operation,
                &format!("feature:{target}"),
                "The resolved Boolean operands refer to the same body feature.",
                "Use two distinct existing or earlier program body features.",
            ));
        }
        *target_feature_id = target.into();
        *tool_feature_id = tool.into();
    }
    Ok(feature)
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
    let mut operation_outputs = BTreeMap::new();
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

    let mut working_colors = snapshot
        .occurrences()
        .map(|o| (o.id(), o.color()))
        .collect::<BTreeMap<_, _>>();
    for (operation_index, operation) in program.operations.iter().enumerate() {
        let operation_name = match operation {
            AssistantCadEditOperation::CreateSketch { .. } => "create_sketch",
            AssistantCadEditOperation::CreatePart { .. } => "create_part",
            AssistantCadEditOperation::AppendFeature { .. } => "append_feature",
            AssistantCadEditOperation::SetDimension { .. } => "set_dimension",
            AssistantCadEditOperation::Delete { .. } => "delete_occurrence",
            AssistantCadEditOperation::Transform { .. } => "transform_occurrence",
            AssistantCadEditOperation::SetColor { .. } => "set_color",
            AssistantCadEditOperation::UpsertClassificationDimension { .. } => {
                "upsert_classification_dimension"
            }
            AssistantCadEditOperation::SetOccurrenceClassification { .. } => {
                "set_occurrence_classification"
            }
            AssistantCadEditOperation::CreateEvaluatorInput { .. } => "create_evaluator_input",
            AssistantCadEditOperation::Copy { .. } => "copy_occurrence",
            AssistantCadEditOperation::LinearPattern { .. } => "linear_pattern_occurrence",
            AssistantCadEditOperation::Mirror { .. } => "mirror_occurrence",
        };
        let selector = match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. }
            | AssistantCadEditOperation::AppendFeature { .. }
            | AssistantCadEditOperation::SetDimension { .. }
            | AssistantCadEditOperation::UpsertClassificationDimension { .. }
            | AssistantCadEditOperation::CreateEvaluatorInput { .. } => None,
            AssistantCadEditOperation::Delete { selector, .. }
            | AssistantCadEditOperation::Transform { selector, .. }
            | AssistantCadEditOperation::SetColor { selector, .. }
            | AssistantCadEditOperation::SetOccurrenceClassification { selector, .. }
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
                let creation_commands = plan_creation(
                    &snapshot,
                    operation,
                    &mut next_definition,
                    &mut next_feature,
                    &mut next_occurrence,
                    &document_target,
                )?;
                if matches!(operation, AssistantCadEditOperation::CreatePart { .. })
                    && let Some(id) = creation_commands.iter().rev().find_map(|command| {
                        if let CanonicalCommand::CreateFeature { id, .. } = command {
                            Some(*id)
                        } else {
                            None
                        }
                    })
                {
                    operation_outputs.insert(operation_index, id);
                }
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::AppendFeature {
                definition_id,
                name,
                feature,
            } => {
                let references_program_output = matches!(
                    feature,
                    AssistantCadBodyFeature::Boolean {
                        target_feature_id: AssistantCadFeatureReference::ProgramOutput(_),
                        ..
                    } | AssistantCadBodyFeature::Boolean {
                        tool_feature_id: AssistantCadFeatureReference::ProgramOutput(_),
                        ..
                    }
                );
                let prefix_candidate = if commands.is_empty() || !references_program_output {
                    None
                } else {
                    Some(
                        document
                            .preview_batch(&CommandBatch::new(commands.clone()))
                            .map_err(|error| {
                                assistant_canonical_rejection(
                                    error,
                                    operation_name,
                                    &document_target,
                                )
                            })?,
                    )
                };
                let planning_snapshot = prefix_candidate.as_ref().unwrap_or(&snapshot);
                let feature = resolve_program_feature_references(
                    feature,
                    &snapshot,
                    &operation_outputs,
                    operation_name,
                )?;
                let definition_id = DefinitionId(*definition_id);
                if planning_snapshot.definition(definition_id).is_none() {
                    return Err(assistant_canonical_rejection(
                        CanonicalError::DefinitionNotFound(definition_id),
                        operation_name,
                        &format!("definition:{}", definition_id.0),
                    ));
                }
                let kind = plan_feature_kind(
                    planning_snapshot,
                    topology_results,
                    definition_id,
                    &feature,
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
                if feature.produces_body_feature_output() {
                    operation_outputs.insert(operation_index, id);
                }
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
            AssistantCadEditOperation::SetColor { color, .. } => {
                for id in targets {
                    working_colors.insert(id, *color);
                    commands.push(CanonicalCommand::SetOccurrenceColor { id, color: *color });
                }
            }
            AssistantCadEditOperation::UpsertClassificationDimension {
                dimension_id,
                name,
                categories,
            } => commands.push(CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(*dimension_id),
                name: name.clone(),
                categories: categories
                    .iter()
                    .map(|category| (ClassificationCategoryId(category.id), category.name.clone()))
                    .collect(),
            }),
            AssistantCadEditOperation::SetOccurrenceClassification {
                dimension_id,
                category_id,
                ..
            } => {
                for occurrence_id in targets {
                    commands.push(CanonicalCommand::SetOccurrenceClassification {
                        occurrence_id,
                        dimension_id: ClassificationDimensionId(*dimension_id),
                        category_id: category_id.map(ClassificationCategoryId),
                    });
                }
            }
            AssistantCadEditOperation::CreateEvaluatorInput {
                node_id,
                name,
                value,
            } => commands.push(CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(*node_id),
                name: name.clone(),
                dimension: Dimension::new(value.to_string(), *value).map_err(|error| {
                    assistant_canonical_rejection(error, operation_name, &format!("node:{node_id}"))
                })?,
                dependencies: Vec::new(),
            }),
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
                    if let Some(color) = working_colors[&id] {
                        commands.push(CanonicalCommand::SetOccurrenceColor {
                            id: occurrence_id,
                            color: Some(color),
                        });
                    }
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
                        if let Some(color) = working_colors[id] {
                            commands.push(CanonicalCommand::SetOccurrenceColor {
                                id: occurrence_id,
                                color: Some(color),
                            });
                        }
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
                    if let Some(color) = working_colors[&id] {
                        commands.push(CanonicalCommand::SetOccurrenceColor {
                            id: occurrence_id,
                            color: Some(color),
                        });
                    }
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
