use crate::diagnostics::{
    AssistantPlanningResult, assistant_canonical_rejection, assistant_planning_rejection,
};
use crate::topology::{
    GeneralFinishKind, assistant_topology_references, plan_topology_finish_kind,
};
use ketchup_core::assistant_sidecar::{AssistantCadBodyFeature, AssistantCadBooleanOperation};
use ketchup_core::document::{
    BooleanOperation, CanonicalError, DefinitionId, Dimension, FeatureId, FeatureKind, LoftSection,
    Snapshot, is_valid_spatial_sweep_path, is_valid_sweep_path,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{ExactResultRegistry, line_arc_profile_bounds};
use ketchup_core::topology::TopologicalElementKind;

pub(crate) fn plan_feature_kind(
    snapshot: &Snapshot,
    topology_results: &ExactResultRegistry,
    definition_id: DefinitionId,
    feature: &AssistantCadBodyFeature,
    operation_name: &str,
) -> AssistantPlanningResult<FeatureKind> {
    Ok(match feature {
        AssistantCadBodyFeature::Boolean {
            operation,
            target_feature_id,
            tool_feature_id,
        } => {
            let target = FeatureId(*target_feature_id);
            let tool = FeatureId(*tool_feature_id);
            let mut input_bounds = [None, None];
            for (index, input) in [target, tool].into_iter().enumerate() {
                let existing = snapshot.feature(input).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::FeatureNotFound(input),
                        operation_name,
                        &format!("feature:{}", input.0),
                    )
                })?;
                if existing.definition_id() != definition_id {
                    return Err(assistant_planning_rejection(
                        "planning.cad_feature_input_ownership_invalid",
                        operation_name,
                        &format!("feature:{}", input.0),
                        "The requested body feature belongs to a different definition.",
                        "Target two supported exact body features in the requested definition.",
                    ));
                }
                let graph = ExactBRepGraph::from_snapshot(
                    snapshot,
                    definition_id,
                    input,
                )
                .map_err(|error| {
                    assistant_planning_rejection(
                        "planning.cad_feature_input_unsupported",
                        operation_name,
                        &format!("feature:{}", input.0),
                        error.to_string(),
                        "Target two supported exact body-producing features in the same definition.",
                    )
                })?;
                input_bounds[index] =
                    graph.producer_bounds_mm().map_err(|error| {
                        assistant_planning_rejection(
                            "planning.cad_feature_input_unsupported",
                            operation_name,
                            &format!("feature:{}", input.0),
                            error.to_string(),
                            "Target two supported exact body-producing features in the same definition.",
                        )
                    })?;
            }
            if matches!(operation, AssistantCadBooleanOperation::Intersect)
                && let [Some(target_bounds), Some(tool_bounds)] = input_bounds
                && (0..3).any(|axis| {
                    target_bounds[0][axis].max(tool_bounds[0][axis])
                        >= target_bounds[1][axis].min(tool_bounds[1][axis])
                })
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_result_empty",
                    operation_name,
                    "feature_inputs",
                    "The bounded Boolean operands do not have a positive-volume intersection.",
                    "Choose two overlapping exact body features for Intersect.",
                ));
            }
            FeatureKind::Boolean {
                operation: match operation {
                    AssistantCadBooleanOperation::Cut => BooleanOperation::Cut,
                    AssistantCadBooleanOperation::Union => BooleanOperation::Union,
                    AssistantCadBooleanOperation::Intersect => BooleanOperation::Intersect,
                },
                target,
                tool,
            }
        }
        AssistantCadBodyFeature::Pocket {
            target_feature_id,
            profile_feature_id,
            depth_mm,
        } => FeatureKind::Pocket {
            target: FeatureId(*target_feature_id),
            profile: FeatureId(*profile_feature_id),
            depth: Dimension::new(depth_mm.to_string(), *depth_mm).map_err(|error| {
                assistant_canonical_rejection(error, operation_name, "feature.depth_mm")
            })?,
        },
        AssistantCadBodyFeature::PlanarOffset {
            profile_feature_id,
            distance_mm,
        } => {
            let profile = FeatureId(*profile_feature_id);
            let source = snapshot.feature(profile).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(profile),
                    operation_name,
                    &format!("feature:{}", profile.0),
                )
            })?;
            if source.definition_id() != definition_id {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_ownership_invalid",
                    operation_name,
                    &format!("feature:{}", profile.0),
                    "The requested Planar Offset profile belongs to a different definition.",
                    "Target the sole supported exact rectangular profile in the requested definition.",
                ));
            }
            let definition = snapshot
                .definition(definition_id)
                .expect("Assistant AppendFeature definition was resolved");
            if snapshot.feature_is_suppressed(profile)
                || definition.feature_ids() != [profile]
                || !matches!(source.kind(), FeatureKind::Profile { .. })
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_unsupported",
                    operation_name,
                    &format!("feature:{}", profile.0),
                    "The requested Planar Offset profile is not supported by exact evaluation.",
                    "Use the sole unsuppressed rectangular profile in the requested definition.",
                ));
            }
            FeatureKind::PlanarOffset {
                profile,
                distance: Dimension::new(distance_mm.to_string(), *distance_mm).map_err(
                    |error| {
                        assistant_canonical_rejection(error, operation_name, "feature.distance_mm")
                    },
                )?,
            }
        }
        AssistantCadBodyFeature::Sweep {
            profile_feature_id,
            path_feature_id,
        } => {
            let profile = FeatureId(*profile_feature_id);
            let path = FeatureId(*path_feature_id);
            let profile_source = snapshot.feature(profile).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(profile),
                    operation_name,
                    &format!("feature:{}", profile.0),
                )
            })?;
            let path_source = snapshot.feature(path).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(path),
                    operation_name,
                    &format!("feature:{}", path.0),
                )
            })?;
            if profile_source.definition_id() != definition_id
                || path_source.definition_id() != definition_id
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_ownership_invalid",
                    operation_name,
                    "feature_inputs",
                    "The requested Sweep inputs belong to a different definition.",
                    "Target a supported profile and path in the requested definition.",
                ));
            }
            let valid_profile = matches!(
                profile_source.kind(),
                FeatureKind::Profile { points_mm } if points_mm.len() >= 3
            ) || matches!(
                profile_source.kind(),
                FeatureKind::SegmentProfile {
                    segments,
                    closed: true,
                } if line_arc_profile_bounds(segments, true).is_some()
            );
            let valid_path = match path_source.kind() {
                FeatureKind::SegmentProfile {
                    segments,
                    closed: false,
                } => is_valid_sweep_path(segments),
                FeatureKind::SpatialPath { segments } => is_valid_spatial_sweep_path(segments),
                _ => false,
            };
            if !valid_profile || !valid_path {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_unsupported",
                    operation_name,
                    "feature_inputs",
                    "The requested Sweep profile or path is not supported by exact evaluation.",
                    "Use a closed polygon or line/arc profile and a bounded open line/arc/cubic path in the same definition.",
                ));
            }
            FeatureKind::Sweep { profile, path }
        }
        AssistantCadBodyFeature::Loft { sections } => {
            let mut loft_sections = Vec::with_capacity(sections.len());
            for section in sections {
                let profile = FeatureId(section.profile_feature_id);
                let source = snapshot.feature(profile).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::FeatureNotFound(profile),
                        operation_name,
                        &format!("feature:{}", profile.0),
                    )
                })?;
                if source.definition_id() != definition_id {
                    return Err(assistant_planning_rejection(
                        "planning.cad_feature_input_ownership_invalid",
                        operation_name,
                        &format!("feature:{}", profile.0),
                        "The requested Loft profile belongs to a different definition.",
                        "Target supported spline profiles in the requested definition.",
                    ));
                }
                if snapshot.feature_is_suppressed(profile)
                    || !matches!(
                        source.kind(),
                        FeatureKind::SplineProfile { control_points_mm }
                            if (4..=64).contains(&control_points_mm.len())
                    )
                {
                    return Err(assistant_planning_rejection(
                        "planning.cad_feature_input_unsupported",
                        operation_name,
                        &format!("feature:{}", profile.0),
                        "The requested Loft profile is not supported by exact evaluation.",
                        "Use an unsuppressed spline profile with 4 to 64 control points.",
                    ));
                }
                loft_sections.push(LoftSection {
                    profile,
                    elevation_mm: section.elevation_mm,
                });
            }
            FeatureKind::Loft {
                sections: loft_sections,
            }
        }
        AssistantCadBodyFeature::TopologyShell {
            target_feature_id,
            removed_face_reference_ids,
            thickness_mm,
        } => {
            let target = FeatureId(*target_feature_id);
            let source = snapshot.feature(target).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(target),
                    operation_name,
                    &format!("feature:{}", target.0),
                )
            })?;
            if source.definition_id() != definition_id {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_ownership_invalid",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Shell target belongs to a different definition.",
                    "Target a supported exact body feature in the requested definition.",
                ));
            }
            if snapshot.feature_is_suppressed(target)
                || ExactBRepGraph::from_snapshot(snapshot, definition_id, target).is_err()
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_unsupported",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Shell target is not supported by exact evaluation.",
                    "Target an unsuppressed supported exact body feature with current host-issued face references.",
                ));
            }
            let available_faces = assistant_topology_references(
                snapshot,
                topology_results,
                TopologicalElementKind::Face,
            );
            let mut removed_faces = Vec::with_capacity(removed_face_reference_ids.len());
            for reference_id in removed_face_reference_ids {
                let matches = available_faces
                    .iter()
                    .copied()
                    .filter(|reference| {
                        reference.definition_id == definition_id
                            && reference.producer_feature_id == target
                            && reference.lineage_digest == *reference_id
                    })
                    .collect::<Vec<_>>();
                let [reference] = matches.as_slice() else {
                    return Err(assistant_planning_rejection(
                        "planning.cad_topology_reference_unavailable",
                        operation_name,
                        &format!("feature:{}", target.0),
                        "A requested Shell face reference is not a unique current host-issued reference for the target.",
                        "Refresh the document context and use only listed topology_face_references for this target.",
                    ));
                };
                removed_faces.push((*reference).clone());
            }
            let thickness =
                Dimension::new(thickness_mm.to_string(), *thickness_mm).map_err(|error| {
                    assistant_canonical_rejection(error, operation_name, "feature.thickness_mm")
                })?;
            plan_topology_finish_kind(GeneralFinishKind::Shell, target, removed_faces, thickness)
                .ok_or_else(|| {
                    assistant_planning_rejection(
                        "planning.cad_topology_reference_set_invalid",
                        operation_name,
                        &format!("feature:{}", target.0),
                        "The requested Shell face reference set is not canonical.",
                        "Use 1 to 64 unique current host-issued face references for one target.",
                    )
                })?
        }
        AssistantCadBodyFeature::TopologyFillet {
            target_feature_id,
            edge_reference_ids,
            radius_mm,
        } => {
            let target = FeatureId(*target_feature_id);
            let source = snapshot.feature(target).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(target),
                    operation_name,
                    &format!("feature:{}", target.0),
                )
            })?;
            if source.definition_id() != definition_id {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_ownership_invalid",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Fillet target belongs to a different definition.",
                    "Target a supported exact body feature in the requested definition.",
                ));
            }
            if snapshot.feature_is_suppressed(target)
                || ExactBRepGraph::from_snapshot(snapshot, definition_id, target).is_err()
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_unsupported",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Fillet target is not supported by exact evaluation.",
                    "Target an unsuppressed supported exact body feature with current host-issued edge references.",
                ));
            }
            let available_edges = assistant_topology_references(
                snapshot,
                topology_results,
                TopologicalElementKind::Edge,
            );
            let mut edges = Vec::with_capacity(edge_reference_ids.len());
            for reference_id in edge_reference_ids {
                let matches = available_edges
                    .iter()
                    .copied()
                    .filter(|reference| {
                        reference.definition_id == definition_id
                            && reference.producer_feature_id == target
                            && reference.lineage_digest == *reference_id
                    })
                    .collect::<Vec<_>>();
                let [reference] = matches.as_slice() else {
                    return Err(assistant_planning_rejection(
                        "planning.cad_topology_reference_unavailable",
                        operation_name,
                        &format!("feature:{}", target.0),
                        "A requested Fillet edge reference is not a unique current host-issued reference for the target.",
                        "Refresh the document context and use only listed topology_edge_references for this target.",
                    ));
                };
                edges.push((*reference).clone());
            }
            let radius = Dimension::new(radius_mm.to_string(), *radius_mm).map_err(|error| {
                assistant_canonical_rejection(error, operation_name, "feature.radius_mm")
            })?;
            plan_topology_finish_kind(GeneralFinishKind::Fillet, target, edges, radius).ok_or_else(
                || {
                    assistant_planning_rejection(
                        "planning.cad_topology_reference_set_invalid",
                        operation_name,
                        &format!("feature:{}", target.0),
                        "The requested Fillet edge reference set is not canonical.",
                        "Use 1 to 64 unique current host-issued edge references for one target.",
                    )
                },
            )?
        }
        AssistantCadBodyFeature::TopologyChamfer {
            target_feature_id,
            edge_reference_ids,
            distance_mm,
        } => {
            let target = FeatureId(*target_feature_id);
            let source = snapshot.feature(target).ok_or_else(|| {
                assistant_canonical_rejection(
                    CanonicalError::FeatureNotFound(target),
                    operation_name,
                    &format!("feature:{}", target.0),
                )
            })?;
            if source.definition_id() != definition_id {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_ownership_invalid",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Chamfer target belongs to a different definition.",
                    "Target a supported exact body feature in the requested definition.",
                ));
            }
            if snapshot.feature_is_suppressed(target)
                || ExactBRepGraph::from_snapshot(snapshot, definition_id, target).is_err()
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_unsupported",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Chamfer target is not supported by exact evaluation.",
                    "Target an unsuppressed supported exact body feature with current host-issued edge references.",
                ));
            }
            let available_edges = assistant_topology_references(
                snapshot,
                topology_results,
                TopologicalElementKind::Edge,
            );
            let mut edges = Vec::with_capacity(edge_reference_ids.len());
            for reference_id in edge_reference_ids {
                let matches = available_edges
                    .iter()
                    .copied()
                    .filter(|reference| {
                        reference.definition_id == definition_id
                            && reference.producer_feature_id == target
                            && reference.lineage_digest == *reference_id
                    })
                    .collect::<Vec<_>>();
                let [reference] = matches.as_slice() else {
                    return Err(assistant_planning_rejection(
                        "planning.cad_topology_reference_unavailable",
                        operation_name,
                        &format!("feature:{}", target.0),
                        "A requested Chamfer edge reference is not a unique current host-issued reference for the target.",
                        "Refresh the document context and use only listed topology_edge_references for this target.",
                    ));
                };
                edges.push((*reference).clone());
            }
            let distance =
                Dimension::new(distance_mm.to_string(), *distance_mm).map_err(|error| {
                    assistant_canonical_rejection(error, operation_name, "feature.distance_mm")
                })?;
            plan_topology_finish_kind(GeneralFinishKind::Chamfer, target, edges, distance)
                .ok_or_else(|| {
                    assistant_planning_rejection(
                        "planning.cad_topology_reference_set_invalid",
                        operation_name,
                        &format!("feature:{}", target.0),
                        "The requested Chamfer edge reference set is not canonical.",
                        "Use 1 to 64 unique current host-issued edge references for one target.",
                    )
                })?
        }
    })
}
