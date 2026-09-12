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
    AssistantAxisSpec, AssistantCadBodyFeature, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadFeatureReference, AssistantCadPartFeature, AssistantCadProgramFeatureOutput,
    AssistantCadProgramFeatureReference, AssistantInstancePath, AssistantInstancePathStep,
    AssistantRejectionDiagnostic, AssistantRejectionPhase, validated_spatial_path_segments,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, ClassificationCategoryId, ClassificationDimensionId,
    CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, InstancePath,
    InstancePathStep, LocalGroupId, LocalOccurrenceId, NodeId, OccurrenceId, ProfileSegment,
    Snapshot, SpatialPathSegment, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactPlanarOffsetRequest, ExactResultRegistry,
};
use ketchup_core::sketch::SketchConstraintId;
use ketchup_core::topology::TopologicalElementKind;
use ketchup_interaction::Vec3;
use std::collections::{BTreeMap, BTreeSet};

const MAX_AXIS_INSTANCE_OCCURRENCES: usize = 10_000;
const MAX_AXIS_INSTANCE_PATH_STEPS: usize = 256;
const MAX_AXIS_INSTANCE_TEXT_BYTES: usize = 4 * 1024 * 1024;

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
    operation_outputs: &BTreeMap<(usize, AssistantCadProgramFeatureOutput), u64>,
    expected_output: AssistantCadProgramFeatureOutput,
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
            .get(&(reference.operation_index as usize, expected_output))
            .copied()
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_program_feature_reference_unavailable",
                    operation,
                    &format!("operation:{}", reference.operation_index),
                    "The referenced earlier operation did not produce the required typed feature.",
                    "Reference a compatible typed output from an earlier operation in this CAD program.",
                )
            }),
    }
}

fn resolve_program_output_reference(
    reference: AssistantCadProgramFeatureReference,
    operation_outputs: &BTreeMap<(usize, AssistantCadProgramFeatureOutput), u64>,
    expected_output: AssistantCadProgramFeatureOutput,
    operation: &str,
) -> AssistantPlanningResult<u64> {
    operation_outputs
        .get(&(reference.operation_index as usize, expected_output))
        .copied()
        .filter(|_| reference.output == expected_output)
        .ok_or_else(|| {
            assistant_planning_rejection(
                "planning.cad_program_feature_reference_unavailable",
                operation,
                &format!("operation:{}", reference.operation_index),
                "The referenced earlier operation did not produce the required typed output.",
                "Reference a compatible typed output from an earlier operation in this CAD program.",
            )
        })
}

fn resolve_assistant_instance_path(
    path: &AssistantInstancePath,
    snapshot: &Snapshot,
) -> Option<InstancePath> {
    let mut resolved = InstancePath::root(OccurrenceId(path.root_occurrence_id));
    let mut owner_definition_id = snapshot
        .occurrence(resolved.root_occurrence())?
        .definition_id();
    for step in &path.steps {
        let (declared_owner, path_step) = match *step {
            AssistantInstancePathStep::Group {
                owner_definition_id,
                local_id,
            } => (
                DefinitionId(owner_definition_id),
                InstancePathStep::Group(LocalGroupId(local_id)),
            ),
            AssistantInstancePathStep::Occurrence {
                owner_definition_id,
                local_id,
            } => (
                DefinitionId(owner_definition_id),
                InstancePathStep::Occurrence(LocalOccurrenceId(local_id)),
            ),
        };
        if declared_owner != owner_definition_id {
            return None;
        }
        resolved = resolved.with_step(path_step);
        owner_definition_id = snapshot
            .resolve_instance_path(&resolved)
            .ok()?
            .definition_id;
    }
    snapshot.resolve_instance_path(&resolved).ok()?;
    Some(resolved)
}

fn transform_axis_to_world(
    transform: Transform,
    origin_mm: [f64; 3],
    direction: [f64; 3],
    requires_similarity: bool,
) -> Option<([f64; 3], [f64; 3])> {
    let matrix = transform.matrix();
    if requires_similarity {
        let columns = [
            [matrix[0], matrix[4], matrix[8]],
            [matrix[1], matrix[5], matrix[9]],
            [matrix[2], matrix[6], matrix[10]],
        ];
        let squared_lengths =
            columns.map(|column| column.iter().map(|value| value * value).sum::<f64>());
        let maximum = squared_lengths.into_iter().fold(0.0_f64, f64::max);
        if !maximum.is_finite()
            || maximum <= f64::EPSILON
            || squared_lengths
                .into_iter()
                .any(|length| (length - maximum).abs() > maximum * 1.0e-9)
            || [(0, 1), (0, 2), (1, 2)].into_iter().any(|(left, right)| {
                let dot = columns[left]
                    .iter()
                    .zip(columns[right])
                    .map(|(left, right)| left * right)
                    .sum::<f64>();
                dot.abs() > maximum * 1.0e-9
            })
        {
            return None;
        }
    }
    let point = |row: usize| {
        matrix[row * 4] * origin_mm[0]
            + matrix[row * 4 + 1] * origin_mm[1]
            + matrix[row * 4 + 2] * origin_mm[2]
            + matrix[row * 4 + 3]
    };
    let vector = |row: usize| {
        matrix[row * 4] * direction[0]
            + matrix[row * 4 + 1] * direction[1]
            + matrix[row * 4 + 2] * direction[2]
    };
    let axis = AssistantAxisSpec::OriginDirection {
        origin_mm: [point(0), point(1), point(2)],
        direction: [vector(0), vector(1), vector(2)],
    };
    axis.origin_and_direction().ok()
}

fn resolve_assistant_axis_spec(
    axis: &AssistantAxisSpec,
    original_snapshot: &Snapshot,
    topology_results: &ExactResultRegistry,
    operation_outputs: &BTreeMap<(usize, AssistantCadProgramFeatureOutput), u64>,
    operations: &[AssistantCadEditOperation],
    operation: &str,
) -> AssistantPlanningResult<([f64; 3], [f64; 3])> {
    if let AssistantAxisSpec::Edge {
        edge_reference_id,
        instance_path,
    } = axis
    {
        let packages = topology_results
            .body_values(original_snapshot)
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_axis_reference_unavailable",
                    operation,
                    "axis.edge_reference_id",
                    "The current exact topology evidence is unavailable.",
                    "Evaluate the current document and use a listed line or circle edge reference.",
                )
            })?;
        let mut matches = Vec::new();
        for package in packages.into_values() {
            let ExactBodyPackage::Graph(package) = package.as_ref() else {
                continue;
            };
            for evidence in &package.edge_evidence {
                let reference = package
                    .topological_references
                    .iter()
                    .filter(|reference| reference.kind == TopologicalElementKind::Edge)
                    .nth(evidence.edge_ordinal as usize);
                if reference.is_some_and(|reference| {
                    reference.lineage_digest == *edge_reference_id
                        && reference.document_id == original_snapshot.document_id()
                        && reference.source_feature_id == reference.producer_feature_id
                        && reference.has_valid_lineage()
                }) && matches!(evidence.curve_kind.as_str(), "line" | "circle")
                    && let (Some(origin_mm), Some(direction)) =
                        (evidence.axis_origin_mm, evidence.unit_axis_direction)
                {
                    matches.push((
                        DefinitionId(package.graph.definition_id),
                        origin_mm,
                        direction,
                        evidence.curve_kind == "circle",
                    ));
                }
            }
        }
        let [(definition_id, origin_mm, direction, requires_similarity)] = matches.as_slice()
        else {
            return Err(assistant_planning_rejection(
                "planning.cad_axis_reference_unavailable",
                operation,
                "axis.edge_reference_id",
                "The edge reference is not one unique current line or circle edge with exact axis evidence.",
                "Refresh exact edge inspection and copy one current line or circle reference_id.",
            ));
        };
        let occurrences = original_snapshot
            .scene_query_bounded(
                MAX_AXIS_INSTANCE_OCCURRENCES,
                MAX_AXIS_INSTANCE_PATH_STEPS,
                MAX_AXIS_INSTANCE_TEXT_BYTES,
            )
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_axis_instance_path_unavailable",
                    operation,
                    "axis.instance_path",
                    "The bounded visible instance index is unavailable.",
                    "Reduce instance nesting or model size, then retry with one current instance_path.",
                )
            })?;
        let resolved_path = if let Some(path) = instance_path {
            resolve_assistant_instance_path(path, original_snapshot)
        } else {
            let mut candidates = occurrences
                .iter()
                .filter(|occurrence| occurrence.visible && occurrence.definition_id == *definition_id)
                .map(|occurrence| occurrence.instance_path.clone());
            let candidate = candidates.next();
            candidate.filter(|_| candidates.next().is_none())
        }
        .ok_or_else(|| {
            assistant_planning_rejection(
                "planning.cad_axis_instance_path_unavailable",
                operation,
                "axis.instance_path",
                "The edge axis does not identify one current visible instance.",
                "Copy the exact instance_path from current instance inspection; repeated definitions require it.",
            )
        })?;
        let resolved = original_snapshot
            .resolve_instance_path(&resolved_path)
            .map_err(|_| {
                assistant_planning_rejection(
                    "planning.cad_axis_instance_path_unavailable",
                    operation,
                    "axis.instance_path",
                    "The edge axis instance path is not current.",
                    "Refresh instance inspection and copy one current instance_path.",
                )
            })?;
        if resolved.definition_id != *definition_id
            || !occurrences
                .iter()
                .any(|occurrence| occurrence.visible && occurrence.instance_path == resolved_path)
        {
            return Err(assistant_planning_rejection(
                "planning.cad_axis_instance_path_unavailable",
                operation,
                "axis.instance_path",
                "The edge reference and visible instance path resolve to different definitions.",
                "Use an instance_path whose definition_id matches the inspected edge.",
            ));
        }
        return transform_axis_to_world(
            resolved.world_transform,
            *origin_mm,
            *direction,
            *requires_similarity,
        )
        .ok_or_else(|| {
            assistant_planning_rejection(
                "planning.cad_axis_instance_transform_invalid",
                operation,
                "axis.instance_path",
                "The visible edge axis could not be transformed into a finite world axis.",
                "Use a current instance with a finite non-degenerate transform.",
            )
        });
    }
    let AssistantAxisSpec::ConstructionAxis { axis } = axis else {
        return axis.origin_and_direction().map_err(|error| {
            assistant_planning_rejection(
                "planning.cad_axis_invalid",
                operation,
                "axis",
                error,
                "Use two distinct bounded points, a finite non-zero origin and direction, or one current exact edge reference.",
            )
        });
    };
    let (feature_id, construction_axis) = match axis {
        AssistantCadFeatureReference::Existing(id) => {
            let feature_id = FeatureId(*id);
            let feature = original_snapshot.feature(feature_id).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(feature_id),
                    operation,
                    &format!("feature:{id}"),
                )
            })?;
            (feature_id, feature.kind())
        }
        AssistantCadFeatureReference::ProgramOutput(reference) => {
            let id = resolve_program_output_reference(
                *reference,
                operation_outputs,
                AssistantCadProgramFeatureOutput::ConstructionFeature,
                operation,
            )?;
            let Some(AssistantCadEditOperation::CreateConstructionAxis {
                origin_mm,
                direction,
                ..
            }) = operations.get(reference.operation_index as usize)
            else {
                return Err(assistant_planning_rejection(
                    "planning.cad_axis_reference_invalid",
                    operation,
                    &format!("feature:{id}"),
                    "The referenced construction output is not an axis.",
                    "Reference a ConstructionFeature output from create_construction_axis.",
                ));
            };
            return Ok((*origin_mm, *direction));
        }
    };
    let FeatureKind::ConstructionAxis {
        origin_mm,
        direction,
    } = construction_axis
    else {
        return Err(assistant_planning_rejection(
            "planning.cad_axis_reference_invalid",
            operation,
            &format!("feature:{}", feature_id.0),
            "The referenced feature is not a construction axis.",
            "Reference an existing ConstructionAxis feature.",
        ));
    };
    Ok((*origin_mm, *direction))
}

fn resolve_program_feature_references(
    feature: &AssistantCadBodyFeature,
    original_snapshot: &Snapshot,
    operation_outputs: &BTreeMap<(usize, AssistantCadProgramFeatureOutput), u64>,
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
            AssistantCadProgramFeatureOutput::BodyFeature,
            operation,
        )?;
        let tool = resolve_program_feature_reference(
            *tool_feature_id,
            original_snapshot,
            operation_outputs,
            AssistantCadProgramFeatureOutput::BodyFeature,
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
    if let AssistantCadBodyFeature::Loft { sections } = &mut feature {
        for section in sections {
            section.profile_feature_id = resolve_program_feature_reference(
                section.profile_feature_id,
                original_snapshot,
                operation_outputs,
                AssistantCadProgramFeatureOutput::SketchFeature,
                operation,
            )?
            .into();
        }
    }
    Ok(feature)
}

fn plan_assistant_construction_creation(
    name: &str,
    feature_suffix: &str,
    feature_kind: FeatureKind,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    rejection_context: (&str, &str),
) -> AssistantPlanningResult<(Vec<CanonicalCommand>, DefinitionId, FeatureId)> {
    let (operation, document_target) = rejection_context;
    let mut exhausted =
        || assistant_canonical_rejection(CanonicalError::IdExhausted, operation, document_target);
    let definition_id = next_definition
        .map(DefinitionId)
        .ok_or_else(&mut exhausted)?;
    *next_definition = definition_id.0.checked_add(1);
    let construction_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = construction_feature_id.0.checked_add(1);
    let occurrence_id = next_occurrence
        .map(OccurrenceId)
        .ok_or_else(&mut exhausted)?;
    *next_occurrence = occurrence_id.0.checked_add(1);
    Ok((
        vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: construction_feature_id,
                definition_id,
                name: format!("{name} {feature_suffix}"),
                kind: feature_kind,
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: name.to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ],
        definition_id,
        construction_feature_id,
    ))
}

fn plan_assistant_helix_thread_creation(
    name: &str,
    profile_segments: Vec<ProfileSegment>,
    path_segments: Vec<SpatialPathSegment>,
    next_definition: &mut Option<u64>,
    next_feature: &mut Option<u64>,
    next_occurrence: &mut Option<u64>,
    rejection_context: (&str, &str),
) -> AssistantPlanningResult<(Vec<CanonicalCommand>, DefinitionId, FeatureId)> {
    let (operation, document_target) = rejection_context;
    let mut exhausted =
        || assistant_canonical_rejection(CanonicalError::IdExhausted, operation, document_target);
    let definition_id = next_definition
        .map(DefinitionId)
        .ok_or_else(&mut exhausted)?;
    *next_definition = definition_id.0.checked_add(1);
    let profile_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = profile_feature_id.0.checked_add(1);
    let path_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = path_feature_id.0.checked_add(1);
    let body_feature_id = next_feature.map(FeatureId).ok_or_else(&mut exhausted)?;
    *next_feature = body_feature_id.0.checked_add(1);
    let occurrence_id = next_occurrence
        .map(OccurrenceId)
        .ok_or_else(&mut exhausted)?;
    *next_occurrence = occurrence_id.0.checked_add(1);
    Ok((
        vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: profile_feature_id,
                definition_id,
                name: format!("{name} profile"),
                kind: FeatureKind::SegmentProfile {
                    segments: profile_segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: path_feature_id,
                definition_id,
                name: format!("{name} path"),
                kind: FeatureKind::SpatialPath {
                    segments: path_segments,
                },
            },
            CanonicalCommand::CreateFeature {
                id: body_feature_id,
                definition_id,
                name: format!("{name} body"),
                kind: FeatureKind::Sweep {
                    profile: profile_feature_id,
                    path: path_feature_id,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: name.to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ],
        definition_id,
        body_feature_id,
    ))
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
            AssistantCadEditOperation::CreateProgramSketch { .. } => "create_program_sketch",
            AssistantCadEditOperation::CreatePart { .. } => "create_part",
            AssistantCadEditOperation::CreateSpatialPath { .. } => "create_spatial_path",
            AssistantCadEditOperation::CreateHelixPath { .. } => "create_helix_path",
            AssistantCadEditOperation::CreateConstructionPoint { .. } => {
                "create_construction_point"
            }
            AssistantCadEditOperation::CreateConstructionAxis { .. } => "create_construction_axis",
            AssistantCadEditOperation::CreateConstructionPlane { .. } => {
                "create_construction_plane"
            }
            AssistantCadEditOperation::CreateHelix { .. } => "create_helix",
            AssistantCadEditOperation::CreateThread { .. } => "create_thread",
            AssistantCadEditOperation::FilletEdges { .. } => "fillet_edges",
            AssistantCadEditOperation::ChamferEdges { .. } => "chamfer_edges",
            AssistantCadEditOperation::AppendFeature { .. } => "append_feature",
            AssistantCadEditOperation::AppendProgramPocket { .. } => "append_program_pocket",
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
            AssistantCadEditOperation::CircularPattern { .. } => "circular_pattern_occurrence",
            AssistantCadEditOperation::Mirror { .. } => "mirror_occurrence",
        };
        let normalized_operation = match operation {
            AssistantCadEditOperation::CreatePart {
                feature: AssistantCadPartFeature::Revolve { axis, .. },
                ..
            } => {
                let (origin_mm, direction) = resolve_assistant_axis_spec(
                    axis,
                    &snapshot,
                    topology_results,
                    &operation_outputs,
                    &program.operations,
                    operation_name,
                )?;
                let mut resolved = operation.clone();
                let AssistantCadEditOperation::CreatePart {
                    feature: AssistantCadPartFeature::Revolve { axis, .. },
                    ..
                } = &mut resolved
                else {
                    unreachable!("matched Revolve part")
                };
                *axis = AssistantAxisSpec::OriginDirection {
                    origin_mm,
                    direction,
                };
                Some(resolved)
            }
            AssistantCadEditOperation::CircularPattern { axis, .. } => {
                let (origin_mm, direction) = resolve_assistant_axis_spec(
                    axis,
                    &snapshot,
                    topology_results,
                    &operation_outputs,
                    &program.operations,
                    operation_name,
                )?;
                let mut resolved = operation.clone();
                let AssistantCadEditOperation::CircularPattern { axis, .. } = &mut resolved else {
                    unreachable!("matched circular pattern")
                };
                *axis = AssistantAxisSpec::OriginDirection {
                    origin_mm,
                    direction,
                };
                Some(resolved)
            }
            AssistantCadEditOperation::FilletEdges {
                definition_id,
                name,
                target_feature_id,
                edge_reference_ids,
                radius_mm,
            } => Some(AssistantCadEditOperation::AppendFeature {
                definition_id: *definition_id,
                name: name.clone(),
                feature: AssistantCadBodyFeature::TopologyFillet {
                    target_feature_id: *target_feature_id,
                    edge_reference_ids: edge_reference_ids.clone(),
                    radius_mm: *radius_mm,
                },
            }),
            AssistantCadEditOperation::ChamferEdges {
                definition_id,
                name,
                target_feature_id,
                edge_reference_ids,
                distance_mm,
            } => Some(AssistantCadEditOperation::AppendFeature {
                definition_id: *definition_id,
                name: name.clone(),
                feature: AssistantCadBodyFeature::TopologyChamfer {
                    target_feature_id: *target_feature_id,
                    edge_reference_ids: edge_reference_ids.clone(),
                    distance_mm: *distance_mm,
                },
            }),
            _ => None,
        };
        let operation = normalized_operation.as_ref().unwrap_or(operation);
        let selector = match operation {
            AssistantCadEditOperation::CreateSketch { .. }
            | AssistantCadEditOperation::CreateProgramSketch { .. }
            | AssistantCadEditOperation::CreatePart { .. }
            | AssistantCadEditOperation::CreateSpatialPath { .. }
            | AssistantCadEditOperation::CreateHelixPath { .. }
            | AssistantCadEditOperation::CreateConstructionPoint { .. }
            | AssistantCadEditOperation::CreateConstructionAxis { .. }
            | AssistantCadEditOperation::CreateConstructionPlane { .. }
            | AssistantCadEditOperation::CreateHelix { .. }
            | AssistantCadEditOperation::CreateThread { .. }
            | AssistantCadEditOperation::FilletEdges { .. }
            | AssistantCadEditOperation::ChamferEdges { .. }
            | AssistantCadEditOperation::AppendFeature { .. }
            | AssistantCadEditOperation::AppendProgramPocket { .. }
            | AssistantCadEditOperation::SetDimension { .. }
            | AssistantCadEditOperation::UpsertClassificationDimension { .. }
            | AssistantCadEditOperation::CreateEvaluatorInput { .. } => None,
            AssistantCadEditOperation::Delete { selector, .. }
            | AssistantCadEditOperation::Transform { selector, .. }
            | AssistantCadEditOperation::SetColor { selector, .. }
            | AssistantCadEditOperation::SetOccurrenceClassification { selector, .. }
            | AssistantCadEditOperation::Copy { selector, .. }
            | AssistantCadEditOperation::LinearPattern { selector, .. }
            | AssistantCadEditOperation::CircularPattern { selector, .. }
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
                for command in &creation_commands {
                    match command {
                        CanonicalCommand::CreateDefinition { id, .. } => {
                            operation_outputs.insert(
                                (
                                    operation_index,
                                    AssistantCadProgramFeatureOutput::Definition,
                                ),
                                id.0,
                            );
                        }
                        CanonicalCommand::CreateFeature {
                            id,
                            kind: ketchup_core::document::FeatureKind::Sketch(_),
                            ..
                        } => {
                            operation_outputs.insert(
                                (
                                    operation_index,
                                    AssistantCadProgramFeatureOutput::SketchFeature,
                                ),
                                id.0,
                            );
                        }
                        CanonicalCommand::CreateFeature { id, kind, .. }
                            if matches!(
                                operation,
                                AssistantCadEditOperation::CreatePart { .. }
                            ) && !matches!(
                                kind,
                                ketchup_core::document::FeatureKind::Workplane(_)
                                    | ketchup_core::document::FeatureKind::Sketch(_)
                            ) =>
                        {
                            operation_outputs.insert(
                                (
                                    operation_index,
                                    AssistantCadProgramFeatureOutput::BodyFeature,
                                ),
                                id.0,
                            );
                        }
                        _ => {}
                    }
                }
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateSpatialPath { name, segments } => {
                let path_segments = validated_spatial_path_segments(segments).map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_spatial_path_invalid",
                        operation_name,
                        &document_target,
                        error,
                        "Use one to 64 finite, connected, non-degenerate 3D line, circular-arc, or cubic-Bezier segments.",
                    )
                })?;
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "path",
                        FeatureKind::SpatialPath {
                            segments: path_segments,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                    ),
                    feature_id.0,
                );
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateHelixPath { name, parameters } => {
                let axis = resolve_assistant_axis_spec(
                    &parameters.axis,
                    &snapshot,
                    topology_results,
                    &operation_outputs,
                    &program.operations,
                    operation_name,
                )?;
                let path_segments =
                    parameters
                        .spatial_path_segments_for_axis(axis)
                        .map_err(|error| {
                            assistant_planning_rejection(
                                "planning.cad_helix_path_invalid",
                                operation_name,
                                &document_target,
                                error,
                                "Use finite bounded Helix parameters and a non-zero 3D axis.",
                            )
                        })?;
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "path",
                        FeatureKind::SpatialPath {
                            segments: path_segments,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                    ),
                    feature_id.0,
                );
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateConstructionPoint { name, position_mm } => {
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "point",
                        FeatureKind::ConstructionPoint {
                            position_mm: *position_mm,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                    ),
                    feature_id.0,
                );
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateConstructionAxis {
                name,
                origin_mm,
                direction,
            } => {
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "axis",
                        FeatureKind::ConstructionAxis {
                            origin_mm: *origin_mm,
                            direction: *direction,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                    ),
                    feature_id.0,
                );
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateConstructionPlane {
                name,
                origin_mm,
                normal,
                x_direction,
            } => {
                let (creation_commands, definition_id, feature_id) =
                    plan_assistant_construction_creation(
                        name,
                        "plane",
                        FeatureKind::ConstructionPlane {
                            origin_mm: *origin_mm,
                            normal: *normal,
                            x_direction: *x_direction,
                        },
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::ConstructionFeature,
                    ),
                    feature_id.0,
                );
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateHelix { name, parameters } => {
                let axis = resolve_assistant_axis_spec(
                    &parameters.axis,
                    &snapshot,
                    topology_results,
                    &operation_outputs,
                    &program.operations,
                    operation_name,
                )?;
                let path_segments =
                    parameters
                        .spatial_path_segments_for_axis(axis)
                        .map_err(|error| {
                            assistant_planning_rejection(
                                "planning.cad_helix_invalid",
                                operation_name,
                                &document_target,
                                error,
                                "Use finite bounded Helix parameters and a non-zero 3D axis.",
                            )
                        })?;
                let wire_radius = (parameters.radius_mm * 0.01)
                    .min(parameters.pitch_mm * 0.2)
                    .clamp(0.002, 0.25);
                let profile_segments = vec![
                    ProfileSegment::CircularArc {
                        start_mm: [-wire_radius, 0.0],
                        end_mm: [wire_radius, 0.0],
                        center_mm: [0.0, 0.0],
                        clockwise: false,
                    },
                    ProfileSegment::CircularArc {
                        start_mm: [wire_radius, 0.0],
                        end_mm: [-wire_radius, 0.0],
                        center_mm: [0.0, 0.0],
                        clockwise: false,
                    },
                ];
                let (creation_commands, definition_id, body_feature_id) =
                    plan_assistant_helix_thread_creation(
                        name,
                        profile_segments,
                        path_segments,
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::BodyFeature,
                    ),
                    body_feature_id.0,
                );
                appended_exact_features.push((definition_id, body_feature_id));
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateThread { name, parameters } => {
                let axis = resolve_assistant_axis_spec(
                    &parameters.helix.axis,
                    &snapshot,
                    topology_results,
                    &operation_outputs,
                    &program.operations,
                    operation_name,
                )?;
                let path_segments = parameters
                    .helix
                    .spatial_path_segments_for_axis(axis)
                    .map_err(|error| {
                        assistant_planning_rejection(
                            "planning.cad_thread_invalid",
                            operation_name,
                            &document_target,
                            error,
                            "Use finite bounded Thread parameters and a non-zero 3D axis.",
                        )
                    })?;
                let profile_segments = parameters.profile_segments().map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_thread_invalid",
                        operation_name,
                        &document_target,
                        error,
                        "Use a positive profile radius smaller than half the pitch.",
                    )
                })?;
                let (creation_commands, definition_id, body_feature_id) =
                    plan_assistant_helix_thread_creation(
                        name,
                        profile_segments,
                        path_segments,
                        &mut next_definition,
                        &mut next_feature,
                        &mut next_occurrence,
                        (operation_name, &document_target),
                    )?;
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::Definition,
                    ),
                    definition_id.0,
                );
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::BodyFeature,
                    ),
                    body_feature_id.0,
                );
                appended_exact_features.push((definition_id, body_feature_id));
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::CreateProgramSketch {
                definition,
                name,
                workplane,
                entities,
                constraints,
            } => {
                let definition_id = resolve_program_output_reference(
                    *definition,
                    &operation_outputs,
                    AssistantCadProgramFeatureOutput::Definition,
                    operation_name,
                )?;
                let planning_snapshot = document
                    .preview_batch(&CommandBatch::new(commands.clone()))
                    .map_err(|error| {
                        assistant_canonical_rejection(error, operation_name, &document_target)
                    })?;
                let workplane = match workplane {
                    ketchup_core::assistant_sidecar::AssistantWorkplaneSpec::ConstructionPlane {
                        plane: AssistantCadFeatureReference::ProgramOutput(reference),
                    } => ketchup_core::assistant_sidecar::AssistantWorkplaneSpec::ConstructionPlane {
                        plane: AssistantCadFeatureReference::Existing(
                            resolve_program_output_reference(
                                *reference,
                                &operation_outputs,
                                AssistantCadProgramFeatureOutput::ConstructionFeature,
                                operation_name,
                            )?,
                        ),
                    },
                    _ => workplane.clone(),
                };
                let resolved = AssistantCadEditOperation::CreateSketch {
                    definition_id,
                    name: name.clone(),
                    workplane,
                    entities: entities.clone(),
                    constraints: constraints.clone(),
                };
                let creation_commands = plan_creation(
                    &planning_snapshot,
                    &resolved,
                    &mut next_definition,
                    &mut next_feature,
                    &mut next_occurrence,
                    &document_target,
                )?;
                let sketch_id = creation_commands.iter().find_map(|command| match command {
                    CanonicalCommand::CreateFeature {
                        id,
                        kind: ketchup_core::document::FeatureKind::Sketch(_),
                        ..
                    } => Some(*id),
                    _ => None,
                });
                if let Some(sketch_id) = sketch_id {
                    operation_outputs.insert(
                        (
                            operation_index,
                            AssistantCadProgramFeatureOutput::SketchFeature,
                        ),
                        sketch_id.0,
                    );
                }
                commands.extend(creation_commands);
            }
            AssistantCadEditOperation::AppendFeature {
                definition_id,
                name,
                feature,
            } => {
                let prefix_candidate = if commands.is_empty() {
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
                let planning_topology = prefix_candidate.as_ref().map(|candidate| {
                    if topology_results.is_bound_to(&snapshot) {
                        ExactResultRegistry::carried_forward(candidate, topology_results)
                    } else {
                        ExactResultRegistry::default()
                    }
                });
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
                    planning_topology.as_ref().unwrap_or(topology_results),
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
                    operation_outputs.insert(
                        (
                            operation_index,
                            AssistantCadProgramFeatureOutput::BodyFeature,
                        ),
                        id.0,
                    );
                }
                if matches!(feature, AssistantCadBodyFeature::PlanarOffset { .. }) {
                    appended_planar_offsets.push((definition_id, id));
                } else {
                    appended_exact_features.push((definition_id, id));
                }
            }
            AssistantCadEditOperation::AppendProgramPocket {
                definition,
                name,
                target_feature,
                profile_feature,
                depth_mm,
            } => {
                let definition_id = DefinitionId(resolve_program_output_reference(
                    *definition,
                    &operation_outputs,
                    AssistantCadProgramFeatureOutput::Definition,
                    operation_name,
                )?);
                let target_feature_id = resolve_program_output_reference(
                    *target_feature,
                    &operation_outputs,
                    AssistantCadProgramFeatureOutput::BodyFeature,
                    operation_name,
                )?;
                let profile_feature_id = resolve_program_output_reference(
                    *profile_feature,
                    &operation_outputs,
                    AssistantCadProgramFeatureOutput::SketchFeature,
                    operation_name,
                )?;
                let planning_snapshot = document
                    .preview_batch(&CommandBatch::new(commands.clone()))
                    .map_err(|error| {
                        assistant_canonical_rejection(error, operation_name, &document_target)
                    })?;
                let feature = AssistantCadBodyFeature::Pocket {
                    target_feature_id,
                    profile_feature_id,
                    depth_mm: *depth_mm,
                };
                let kind = plan_feature_kind(
                    &planning_snapshot,
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
                operation_outputs.insert(
                    (
                        operation_index,
                        AssistantCadProgramFeatureOutput::BodyFeature,
                    ),
                    id.0,
                );
                appended_exact_features.push((definition_id, id));
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
            AssistantCadEditOperation::CircularPattern {
                instances,
                axis,
                angle_step_degrees,
                ..
            } => {
                let (origin_mm, direction) = axis
                    .origin_and_direction()
                    .expect("referenced circular pattern axis was normalized before planning");
                let centre = Vec3::new(origin_mm[0], origin_mm[1], origin_mm[2]);
                let direction = Vec3::new(direction[0], direction[1], direction[2]);
                for instance in 1..*instances {
                    let world_rotation = world_axis_rotation_transform(
                        centre,
                        direction,
                        angle_step_degrees * f64::from(instance),
                    )
                    .map_err(|_| {
                        assistant_planning_rejection(
                            "planning.cad_pattern_invalid",
                            operation_name,
                            "occurrence_selection",
                            "The requested circular pattern transform could not be represented.",
                            "Use a valid shared axis, finite angle step, and instance count.",
                        )
                    })?;
                    for id in &targets {
                        let source = snapshot
                            .occurrence(*id)
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
                                    "Refresh the document context and retry the circular pattern.",
                                )
                            })?;
                        let transform = rotation_in_parent_space(
                            world_rotation,
                            parent_transform,
                            working_transforms[id],
                        )
                        .ok_or_else(|| {
                            assistant_planning_rejection(
                                "planning.cad_pattern_invalid",
                                operation_name,
                                &format!("occurrence:{}", id.0),
                                "The circular pattern could not be represented in the occurrence parent.",
                                "Use a finite invertible parent transform and shared axis.",
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
            AssistantCadEditOperation::FilletEdges { .. }
            | AssistantCadEditOperation::ChamferEdges { .. } => {
                unreachable!("direct edge finishes are normalized before planning")
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
