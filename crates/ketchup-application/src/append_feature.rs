use crate::diagnostics::{
    AssistantPlanningResult, assistant_canonical_rejection, assistant_planning_rejection,
};
use crate::topology::{
    GeneralFinishKind, assistant_topology_references, plan_topology_advanced_chamfer_kind,
    plan_topology_finish_kind, plan_topology_shell_kind, plan_topology_variable_fillet_kind,
};
use ketchup_core::assistant_sidecar::{
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadChamferMode,
    AssistantCadLoftContinuity, AssistantCadShellDirection, AssistantCadSurfaceBodySource,
};
use ketchup_core::document::{
    BooleanOperation, CanonicalError, ChamferEdgeSide, ChamferMode, DefinitionId, Dimension,
    FeatureId, FeatureKind, FilletRadiusStation, LoftContinuity, LoftSection, ShellDirection,
    Snapshot, SurfaceBodySpec, WeldmentJointSpec, WeldmentMemberSpec, is_valid_spatial_sweep_path,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{ExactResultRegistry, accepts_planar_offset_solved_region};
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
            let target = FeatureId(
                target_feature_id
                    .existing_id()
                    .expect("Assistant Boolean references are resolved before feature planning"),
            );
            let tool = FeatureId(
                tool_feature_id
                    .existing_id()
                    .expect("Assistant Boolean references are resolved before feature planning"),
            );
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
            let supported = match source.kind() {
                FeatureKind::Profile { .. } => snapshot
                    .definition(definition_id)
                    .is_some_and(|definition| definition.feature_ids() == [profile]),
                FeatureKind::Sketch(sketch) => sketch.solved_regions().is_ok_and(|regions| {
                    matches!(regions.as_slice(), [region] if accepts_planar_offset_solved_region(region, *distance_mm))
                }),
                _ => false,
            };
            if snapshot.feature_is_suppressed(profile) || !supported {
                return Err(assistant_planning_rejection(
                    "planning.cad_feature_input_unsupported",
                    operation_name,
                    &format!("feature:{}", profile.0),
                    "The requested Planar Offset profile is not one supported closed region.",
                    "Use an unsuppressed legacy rectangle or a sketch with exactly one compatible closed region.",
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
            FeatureKind::Sweep { profile, path }
        }
        AssistantCadBodyFeature::WeldmentMember {
            profile_feature_id,
            path_feature_id,
            orientation_degrees,
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
                || snapshot.feature_is_suppressed(profile)
                || snapshot.feature_is_suppressed(path)
            {
                return Err(assistant_planning_rejection(
                    "planning.cad_weldment_member_input_invalid",
                    operation_name,
                    "feature_inputs",
                    "The requested weldment profile or spatial path is unavailable in the target definition.",
                    "Use an unsuppressed closed profile and bounded SpatialPath in the same definition.",
                ));
            }
            FeatureKind::WeldmentMember(WeldmentMemberSpec {
                profile,
                path,
                orientation_degrees: *orientation_degrees,
            })
        }
        AssistantCadBodyFeature::WeldmentJoint {
            first_member_id,
            second_member_id,
            policy,
            primary,
        } => {
            let first_member = FeatureId(first_member_id.existing_id().expect(
                "Assistant weldment joint references are resolved before feature planning",
            ));
            let second_member = FeatureId(second_member_id.existing_id().expect(
                "Assistant weldment joint references are resolved before feature planning",
            ));
            for member_id in [first_member, second_member] {
                let member = snapshot.feature(member_id).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::FeatureNotFound(member_id),
                        operation_name,
                        &format!("feature:{}", member_id.0),
                    )
                })?;
                if member.definition_id() != definition_id
                    || snapshot.feature_is_suppressed(member_id)
                    || !matches!(member.kind(), FeatureKind::WeldmentMember(_))
                {
                    return Err(assistant_planning_rejection(
                        "planning.cad_weldment_joint_input_invalid",
                        operation_name,
                        &format!("feature:{}", member_id.0),
                        "The requested weldment joint input is not an available member in the target definition.",
                        "Use two distinct unsuppressed weldment members in the same definition.",
                    ));
                }
            }
            FeatureKind::WeldmentJoint(WeldmentJointSpec {
                first_member,
                second_member,
                policy: (*policy).into(),
                primary: (*primary).into(),
            })
        }
        AssistantCadBodyFeature::Loft {
            sections,
            guide_feature_id,
            continuity,
        } => {
            let mut loft_sections = Vec::with_capacity(sections.len());
            for section in sections {
                let profile = FeatureId(
                    section
                        .profile_feature_id
                        .existing_id()
                        .expect("Assistant Loft references are resolved before feature planning"),
                );
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
                        "Target supported closed profiles in the requested definition.",
                    ));
                }
                let supported = match source.kind() {
                    FeatureKind::SplineProfile { control_points_mm } => {
                        (4..=64).contains(&control_points_mm.len())
                    }
                    FeatureKind::Sketch(sketch) => sketch
                        .solved_regions()
                        .is_ok_and(|regions| matches!(regions.as_slice(), [_])),
                    _ => false,
                };
                if snapshot.feature_is_suppressed(profile) || !supported {
                    return Err(assistant_planning_rejection(
                        "planning.cad_feature_input_unsupported",
                        operation_name,
                        &format!("feature:{}", profile.0),
                        "The requested Loft profile is not one supported closed region.",
                        "Use an unsuppressed spline profile or a sketch with exactly one closed region.",
                    ));
                }
                loft_sections.push(LoftSection {
                    profile,
                    elevation_mm: section.elevation_mm,
                });
            }
            let guide = guide_feature_id.map(|guide| {
                FeatureId(
                    guide
                        .existing_id()
                        .expect("Assistant Loft guide is resolved before feature planning"),
                )
            });
            if let Some(guide) = guide {
                let source = snapshot.feature(guide).ok_or_else(|| {
                    assistant_canonical_rejection(
                        CanonicalError::FeatureNotFound(guide),
                        operation_name,
                        &format!("feature:{}", guide.0),
                    )
                })?;
                if source.definition_id() != definition_id
                    || snapshot.feature_is_suppressed(guide)
                    || !matches!(
                        source.kind(),
                        FeatureKind::SpatialPath { segments }
                            if is_valid_spatial_sweep_path(segments)
                    )
                {
                    return Err(assistant_planning_rejection(
                        "planning.cad_feature_input_unsupported",
                        operation_name,
                        &format!("feature:{}", guide.0),
                        "The requested Loft guide is not one bounded C1 spatial path in the same definition.",
                        "Use an unsuppressed line/arc/cubic SpatialPath that satisfies the exact path contract.",
                    ));
                }
            }
            FeatureKind::Loft {
                sections: loft_sections,
                guide,
                continuity: match continuity {
                    AssistantCadLoftContinuity::Position => LoftContinuity::Position,
                    AssistantCadLoftContinuity::Tangent => LoftContinuity::Tangent,
                    AssistantCadLoftContinuity::Curvature => LoftContinuity::Curvature,
                },
            }
        }
        AssistantCadBodyFeature::SurfaceBody { source } => {
            FeatureKind::SurfaceBody(match source {
                AssistantCadSurfaceBodySource::Planar { profile_feature_id } => {
                    SurfaceBodySpec::Planar {
                        profile: FeatureId(profile_feature_id.existing_id().expect(
                            "Assistant planar SurfaceBody reference is resolved before feature planning",
                        )),
                    }
                }
                AssistantCadSurfaceBodySource::Loft {
                    sections,
                    guide_feature_id,
                    continuity,
                } => SurfaceBodySpec::Loft {
                    sections: sections
                        .iter()
                        .map(|section| LoftSection {
                            profile: FeatureId(section.profile_feature_id.existing_id().expect(
                                "Assistant loft SurfaceBody references are resolved before feature planning",
                            )),
                            elevation_mm: section.elevation_mm,
                        })
                        .collect(),
                    guide: guide_feature_id.map(|guide| {
                        FeatureId(guide.existing_id().expect(
                            "Assistant loft SurfaceBody guide is resolved before feature planning",
                        ))
                    }),
                    continuity: match continuity {
                        AssistantCadLoftContinuity::Position => LoftContinuity::Position,
                        AssistantCadLoftContinuity::Tangent => LoftContinuity::Tangent,
                        AssistantCadLoftContinuity::Curvature => LoftContinuity::Curvature,
                    },
                },
            })
        }
        AssistantCadBodyFeature::SurfaceTrim {
            target_feature_id,
            cutter_feature_id,
        } => FeatureKind::SurfaceTrim {
            target: FeatureId(target_feature_id.existing_id().expect(
                "Assistant SurfaceTrim target is resolved before feature planning",
            )),
            cutter: FeatureId(cutter_feature_id.existing_id().expect(
                "Assistant SurfaceTrim cutter is resolved before feature planning",
            )),
        },
        AssistantCadBodyFeature::SurfaceExtend {
            target_feature_id,
            distance_mm,
        } => FeatureKind::SurfaceExtend {
            target: FeatureId(target_feature_id.existing_id().expect(
                "Assistant SurfaceExtend target is resolved before feature planning",
            )),
            distance: Dimension::new(distance_mm.to_string(), *distance_mm).map_err(|error| {
                assistant_canonical_rejection(error, operation_name, "feature.distance_mm")
            })?,
        },
        AssistantCadBodyFeature::SurfaceKnit {
            surface_feature_ids,
            tolerance_mm,
            make_solid,
        } => {
            let mut surfaces = surface_feature_ids
                .iter()
                .map(|reference| {
                    FeatureId(reference.existing_id().expect(
                        "Assistant SurfaceKnit references are resolved before feature planning",
                    ))
                })
                .collect::<Vec<_>>();
            surfaces.sort_unstable();
            FeatureKind::SurfaceKnit {
                surfaces,
                tolerance: Dimension::new(tolerance_mm.to_string(), *tolerance_mm).map_err(
                    |error| {
                        assistant_canonical_rejection(
                            error,
                            operation_name,
                            "feature.tolerance_mm",
                        )
                    },
                )?,
                make_solid: *make_solid,
            }
        }
        AssistantCadBodyFeature::SurfaceThicken {
            target_feature_id,
            thickness_mm,
            direction,
        } => FeatureKind::SurfaceThicken {
            target: FeatureId(target_feature_id.existing_id().expect(
                "Assistant SurfaceThicken target is resolved before feature planning",
            )),
            thickness: Dimension::new(thickness_mm.to_string(), *thickness_mm).map_err(|error| {
                assistant_canonical_rejection(error, operation_name, "feature.thickness_mm")
            })?,
            direction: match direction {
                AssistantCadShellDirection::Inward => ShellDirection::Inward,
                AssistantCadShellDirection::Outward => ShellDirection::Outward,
                AssistantCadShellDirection::Symmetric => ShellDirection::Symmetric,
            },
        },
        AssistantCadBodyFeature::SheetMetal { .. } => {
            let spec = feature.sheet_metal_spec().ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_sheet_metal_invalid",
                    operation_name,
                    "feature",
                    "The requested sheet-metal dimensions or bend parameters are invalid.",
                    "Use bounded positive dimensions, a K-factor from zero to one, canonical unique boundary edges, and no adjacent flanges without corner relief.",
                )
            })?;
            spec.validate().map_err(|error| {
                assistant_planning_rejection(
                    "planning.cad_sheet_metal_invalid",
                    operation_name,
                    "feature",
                    error.to_string(),
                    "Use bounded positive dimensions, a K-factor from zero to one, canonical unique boundary edges, and no adjacent flanges without corner relief.",
                )
            })?;
            FeatureKind::SheetMetal(spec)
        }
        AssistantCadBodyFeature::TopologyShell {
            target_feature_id,
            removed_face_reference_ids,
            thickness_mm,
            direction,
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
            plan_topology_shell_kind(
                target,
                removed_faces,
                thickness,
                match direction {
                    AssistantCadShellDirection::Inward => ShellDirection::Inward,
                    AssistantCadShellDirection::Outward => ShellDirection::Outward,
                    AssistantCadShellDirection::Symmetric => ShellDirection::Symmetric,
                },
            )
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_topology_reference_set_invalid",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Shell face reference set is not canonical.",
                    "Use zero to 64 unique current host-issued face references for one target.",
                )
            })?
        }
        AssistantCadBodyFeature::TopologyFillet {
            target_feature_id,
            edge_reference_ids,
            radius_mm,
            radius_stations,
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
            let radius_stations = radius_stations
                .iter()
                .enumerate()
                .map(|(index, station)| {
                    Ok(FilletRadiusStation {
                        position: station.position,
                        radius: Dimension::new(station.radius_mm.to_string(), station.radius_mm)
                            .map_err(|error| {
                                assistant_canonical_rejection(
                                    error,
                                    operation_name,
                                    &format!("feature.radius_stations.{index}.radius_mm"),
                                )
                            })?,
                    })
                })
                .collect::<AssistantPlanningResult<Vec<_>>>()?;
            plan_topology_variable_fillet_kind(target, edges, radius, radius_stations).ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_topology_reference_set_invalid",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Fillet edge reference set or radius profile is not canonical.",
                    "Use 1 to 64 unique current host-issued edge references and a bounded ordered radius profile ending at position 1.",
                )
            })?
        }
        AssistantCadBodyFeature::TopologyChamfer {
            target_feature_id,
            edge_reference_ids,
            distance_mm,
            mode,
            side_face_reference_ids,
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
            match mode {
                AssistantCadChamferMode::Symmetric => {
                    plan_topology_finish_kind(GeneralFinishKind::Chamfer, target, edges, distance)
                }
                AssistantCadChamferMode::TwoDistance { second_distance_mm }
                | AssistantCadChamferMode::DistanceAngle {
                    angle_degrees: second_distance_mm,
                } => {
                    let available_faces = assistant_topology_references(
                        snapshot,
                        topology_results,
                        TopologicalElementKind::Face,
                    );
                    let mut edge_sides = Vec::with_capacity(edges.len());
                    for (edge, reference_id) in edges.into_iter().zip(side_face_reference_ids) {
                        let matches = available_faces
                            .iter()
                            .copied()
                            .filter(|reference| {
                                reference.definition_id == definition_id
                                    && reference.producer_feature_id == target
                                    && reference.lineage_digest == *reference_id
                            })
                            .collect::<Vec<_>>();
                        let [side_face] = matches.as_slice() else {
                            return Err(assistant_planning_rejection(
                                "planning.cad_topology_reference_unavailable",
                                operation_name,
                                &format!("feature:{}", target.0),
                                "A requested Chamfer side-face reference is not a unique current host-issued reference for the target.",
                                "Refresh the document context and use only listed topology_face_references for this target.",
                            ));
                        };
                        edge_sides.push(ChamferEdgeSide {
                            edge,
                            side_face: (*side_face).clone(),
                        });
                    }
                    let mode = match mode {
                        AssistantCadChamferMode::TwoDistance { .. } => ChamferMode::TwoDistance {
                            second_distance: Dimension::new(
                                second_distance_mm.to_string(),
                                *second_distance_mm,
                            )
                            .map_err(|error| {
                                assistant_canonical_rejection(
                                    error,
                                    operation_name,
                                    "feature.mode.second_distance_mm",
                                )
                            })?,
                        },
                        AssistantCadChamferMode::DistanceAngle { .. } => {
                            ChamferMode::DistanceAngle {
                                angle_degrees: *second_distance_mm,
                            }
                        }
                        AssistantCadChamferMode::Symmetric => unreachable!(),
                    };
                    plan_topology_advanced_chamfer_kind(target, edge_sides, distance, mode)
                }
            }
            .ok_or_else(|| {
                assistant_planning_rejection(
                    "planning.cad_topology_reference_set_invalid",
                    operation_name,
                    &format!("feature:{}", target.0),
                    "The requested Chamfer edge/side-face set is not canonical.",
                    "Use 1 to 64 current host-issued edge references and one adjacent side-face reference per edge for advanced modes.",
                )
            })?
        }
    })
}
