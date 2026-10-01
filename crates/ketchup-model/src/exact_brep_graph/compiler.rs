use super::*;
use ketchup_geometry::linalg::{cross, dot, normalize_within, sub};

pub(super) struct GraphCompiler<'a> {
    pub(super) snapshot: &'a Snapshot,
    pub(super) definition_id: DefinitionId,
    pub(super) profiles: Vec<ExactBRepProfile>,
    pub(super) nodes: Vec<ExactBRepNode>,
    pub(super) node_bounds: Vec<Option<[[f64; 3]; 2]>>,
    pub(super) compiled_bodies: BTreeMap<FeatureId, ExactBRepNodeId>,
    pub(super) profile_ids:
        BTreeMap<(FeatureId, Option<SketchRegionId>, [u64; 12]), ExactBRepProfileId>,
    pub(super) visiting: BTreeSet<FeatureId>,
}

impl<'a> GraphCompiler<'a> {
    pub(super) fn new(snapshot: &'a Snapshot, definition_id: DefinitionId) -> Self {
        Self {
            snapshot,
            definition_id,
            profiles: Vec::new(),
            nodes: Vec::new(),
            node_bounds: Vec::new(),
            compiled_bodies: BTreeMap::new(),
            profile_ids: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    pub(super) fn compile_body(
        &mut self,
        feature_id: FeatureId,
    ) -> Result<ExactBRepNodeId, ExactBRepGraphError> {
        // Preserve target-before-tool DFS order (and graph fingerprints) without host recursion.
        let mut pending = vec![(feature_id, false)];
        while let Some((id, expanded)) = pending.pop() {
            if self.compiled_bodies.contains_key(&id) {
                continue;
            }
            if expanded {
                self.compile_node(id)?;
                continue;
            }
            if self.visiting.contains(&id) {
                return Err(ExactBRepGraphError::DependencyCycle(id));
            }
            if self.nodes.len() + self.visiting.len() >= MAX_EXACT_BREP_GRAPH_NODES {
                return Err(ExactBRepGraphError::ResourceLimit);
            }
            let feature = self
                .snapshot
                .feature(id)
                .filter(|feature| feature.definition_id() == self.definition_id)
                .ok_or(ExactBRepGraphError::FeatureNotFound(id))?;
            if self.snapshot.feature_is_suppressed(id) {
                return Err(ExactBRepGraphError::SuppressedFeature(id));
            }
            self.visiting.insert(id);
            pending.push((id, true));
            match feature.kind() {
                FeatureKind::Pad(spec) => {
                    pending.extend(spec.operation.target().map(|target| (target, false)))
                }
                FeatureKind::Shell { target, .. }
                | FeatureKind::EdgeFinish { target, .. }
                | FeatureKind::FaceOffset { target, .. }
                | FeatureKind::SurfaceExtend { target, .. }
                | FeatureKind::SurfaceThicken { target, .. }
                | FeatureKind::RigidTransform { target, .. } => pending.push((*target, false)),
                FeatureKind::Boolean { target, tool, .. }
                | FeatureKind::SurfaceTrim {
                    target,
                    cutter: tool,
                } => {
                    pending.push((*tool, false));
                    pending.push((*target, false));
                }
                FeatureKind::SurfaceKnit { surfaces, .. } => {
                    for surface in surfaces.iter().rev() {
                        pending.push((*surface, false));
                    }
                }
                FeatureKind::WeldmentJoint(spec) => {
                    pending.push((spec.second_member, false));
                    pending.push((spec.first_member, false));
                }
                _ => {}
            }
        }
        self.body_id(feature_id)
    }

    pub(super) fn body_id(
        &self,
        feature_id: FeatureId,
    ) -> Result<ExactBRepNodeId, ExactBRepGraphError> {
        self.compiled_bodies
            .get(&feature_id)
            .copied()
            .ok_or(ExactBRepGraphError::InvalidDependencyGraph)
    }

    pub(super) fn compile_node(
        &mut self,
        feature_id: FeatureId,
    ) -> Result<ExactBRepNodeId, ExactBRepGraphError> {
        let feature = self
            .snapshot
            .feature(feature_id)
            .filter(|feature| feature.definition_id() == self.definition_id)
            .ok_or(ExactBRepGraphError::FeatureNotFound(feature_id))?;
        if self.snapshot.feature_is_suppressed(feature_id) {
            return Err(ExactBRepGraphError::SuppressedFeature(feature_id));
        }
        let operation = match feature.kind() {
            FeatureKind::Pad(spec) => {
                if !matches!(spec.extent, FeatureExtent::Blind(_))
                    && let Some(support) = spec.operation.support()
                    && self
                        .snapshot
                        .exact_reference_by_lineage(&support.lineage_digest)
                        != Some(support)
                {
                    return Err(ExactBRepGraphError::UnresolvedExtent);
                }
                let (profile, origin_mm, direction) =
                    self.compile_pad_profile(spec.profile, spec.direction)?;
                match &spec.operation {
                    PadOperation::NewBody => {
                        let interval =
                            self.resolve_extent(origin_mm, direction, &spec.extent, None)?;
                        ExactBRepOperation::Extrude {
                            profile,
                            distance_bits: positive_distance(
                                interval.length_mm(),
                                self.snapshot.tolerance().linear_mm(),
                            )?,
                            interval,
                        }
                    }
                    PadOperation::Cut { target, start } => {
                        let target = self.body_id(*target)?;
                        let target_bounds = self.node_bounds[target.0 as usize];
                        let interval = match (start, &spec.extent) {
                            // Measured back into the target from the face the
                            // direction points out of; the floor must stay inside.
                            (CutStart::TargetFace, FeatureExtent::Blind(depth)) => {
                                let target_bounds =
                                    target_bounds.ok_or(ExactBRepGraphError::UnresolvedExtent)?;
                                let [_, top_mm] = projected_bounds(
                                    origin_mm,
                                    direction,
                                    target_bounds,
                                    self.snapshot.tolerance().linear_mm(),
                                )?;
                                let interval = linear_interval(
                                    direction,
                                    top_mm - depth.millimetres(),
                                    top_mm,
                                    self.snapshot.tolerance().linear_mm(),
                                )?;
                                validate_blind_interval(
                                    origin_mm,
                                    interval,
                                    interval.start_mm(),
                                    target_bounds,
                                    self.snapshot.tolerance().linear_mm(),
                                )?;
                                interval
                            }
                            (CutStart::TargetFace, _) => {
                                return Err(ExactBRepGraphError::InvalidParameter);
                            }
                            (CutStart::ProfilePlane | CutStart::Support(_), extent) => {
                                self.resolve_extent(origin_mm, direction, extent, target_bounds)?
                            }
                        };
                        ExactBRepOperation::ProfileCut {
                            target,
                            profile,
                            // A through cut is sized against the evaluated target.
                            depth_bits: (spec.extent != FeatureExtent::ThroughAll)
                                .then(|| {
                                    positive_distance(
                                        interval.length_mm(),
                                        self.snapshot.tolerance().linear_mm(),
                                    )
                                })
                                .transpose()?,
                            interval,
                            support_lineage_digest: spec
                                .operation
                                .support()
                                .map(|support| support.lineage_digest.clone()),
                            tool_name: Some(feature.name().to_owned()),
                        }
                    }
                }
            }
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => ExactBRepOperation::Boolean {
                operation: (*operation).into(),
                target: self.body_id(*target)?,
                tool: self.body_id(*tool)?,
                tool_name: Some(feature.name().to_owned()),
            },
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
                direction,
            } => {
                let recorded = removed_faces
                    .iter()
                    .filter_map(FaceRef::topological)
                    .cloned()
                    .collect::<Vec<_>>();
                let profile_faces = removed_faces
                    .iter()
                    .filter_map(FaceRef::named)
                    .map(exact_profile_face_reference)
                    .collect::<Vec<_>>();
                ExactBRepOperation::Shell {
                    target: self.body_id(*target)?,
                    removed_faces: if recorded.is_empty() {
                        Vec::new()
                    } else {
                        topology_selectors(&recorded, TopologicalElementKind::Face, *target)?
                    },
                    thickness_bits: positive_distance(
                        thickness.millimetres(),
                        self.snapshot.tolerance().linear_mm(),
                    )?,
                    direction: (*direction).into(),
                    name: (!profile_faces.is_empty()).then(|| feature.name().to_owned()),
                    profile_faces,
                }
            }
            FeatureKind::EdgeFinish {
                target,
                edges,
                kind,
                amount,
                fillet_radius_stations,
                chamfer_mode,
                chamfer_edge_sides,
            } => ExactBRepOperation::EdgeFinish {
                target: self.body_id(*target)?,
                edges: match EdgeRef::all_topological(edges) {
                    Some(recorded) => {
                        topology_selectors(&recorded, TopologicalElementKind::Edge, *target)?
                    }
                    None => Vec::new(),
                },
                profile_edges: edges
                    .iter()
                    .filter_map(EdgeRef::named)
                    .map(|edge| ExactBRepProfileEdgeReference {
                        first: exact_profile_face_reference(&edge.first),
                        second: exact_profile_face_reference(&edge.second),
                    })
                    .collect(),
                kind: (*kind).into(),
                amount_bits: positive_distance(
                    amount.millimetres(),
                    self.snapshot.tolerance().linear_mm(),
                )?,
                fillet_radius_stations: fillet_radius_stations
                    .iter()
                    .map(|station| {
                        Ok(ExactBRepFilletRadiusStation {
                            position_bits: station.position.to_bits(),
                            radius_bits: positive_distance(
                                station.radius.millimetres(),
                                self.snapshot.tolerance().linear_mm(),
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, ExactBRepGraphError>>()?,
                chamfer_mode: match chamfer_mode {
                    ChamferMode::Symmetric => ExactBRepChamferMode::Symmetric,
                    ChamferMode::TwoDistance { second_distance } => {
                        ExactBRepChamferMode::TwoDistance {
                            second_distance_bits: positive_distance(
                                second_distance.millimetres(),
                                self.snapshot.tolerance().linear_mm(),
                            )?,
                        }
                    }
                    ChamferMode::DistanceAngle { angle_degrees } => {
                        if !angle_degrees.is_finite()
                            || *angle_degrees <= 0.1
                            || *angle_degrees >= 89.9
                        {
                            return Err(ExactBRepGraphError::InvalidParameter);
                        }
                        ExactBRepChamferMode::DistanceAngle {
                            angle_degrees_bits: angle_degrees.to_bits(),
                        }
                    }
                },
                chamfer_edge_sides: chamfer_edge_sides
                    .iter()
                    .map(|selection| {
                        let edge = topology_selectors(
                            std::slice::from_ref(&selection.edge),
                            TopologicalElementKind::Edge,
                            *target,
                        )?
                        .pop()
                        .ok_or(ExactBRepGraphError::InvalidTopologySelector)?;
                        let side_face = topology_selectors(
                            std::slice::from_ref(&selection.side_face),
                            TopologicalElementKind::Face,
                            *target,
                        )?
                        .pop()
                        .ok_or(ExactBRepGraphError::InvalidTopologySelector)?;
                        Ok(ExactBRepChamferEdgeSide { edge, side_face })
                    })
                    .collect::<Result<Vec<_>, ExactBRepGraphError>>()?,
            },
            FeatureKind::FaceOffset {
                target,
                face,
                distance,
            } => ExactBRepOperation::FaceOffset {
                target: self.body_id(*target)?,
                face: face
                    .topological()
                    .map(|face| {
                        topology_selectors(
                            std::slice::from_ref(face),
                            TopologicalElementKind::Face,
                            *target,
                        )?
                        .pop()
                        .ok_or(ExactBRepGraphError::InvalidTopologySelector)
                    })
                    .transpose()?,
                profile_face: face.named().map(exact_profile_face_reference),
                distance_bits: signed_distance(
                    distance.millimetres(),
                    self.snapshot.tolerance().linear_mm(),
                )?,
            },
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                let compiled_profile = match self
                    .snapshot
                    .feature(*profile)
                    .map(|feature| feature.kind())
                {
                    Some(FeatureKind::Sketch(sketch)) => {
                        let regions = sketch.solved_regions().map_err(|error| {
                            ExactBRepGraphError::UnsolvedProfile(*profile, error)
                        })?;
                        let [region] = regions.as_slice() else {
                            return Err(ExactBRepGraphError::UnsupportedProfile(*profile));
                        };
                        self.compile_sketch_profile(
                            *profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    _ => self.compile_profile(*profile, None, identity_frame())?,
                };
                ExactBRepOperation::Revolve {
                    profile: compiled_profile,
                    axis_start_bits: axis_start_mm.map(f64::to_bits),
                    axis_end_bits: axis_end_mm.map(f64::to_bits),
                    angle_degrees_bits: angle_degrees.to_bits(),
                }
            }
            FeatureKind::PlanarOffset { profile, distance } => {
                let compiled_profile = match self
                    .snapshot
                    .feature(*profile)
                    .map(|feature| feature.kind())
                {
                    Some(FeatureKind::Sketch(sketch)) => {
                        let regions = sketch.solved_regions().map_err(|error| {
                            ExactBRepGraphError::UnsolvedProfile(*profile, error)
                        })?;
                        let [region] = regions.as_slice() else {
                            return Err(ExactBRepGraphError::UnsupportedProfile(*profile));
                        };
                        self.compile_sketch_profile(
                            *profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    _ => self.compile_profile(*profile, None, identity_frame())?,
                };
                ExactBRepOperation::PlanarOffset {
                    profile: compiled_profile,
                    distance_bits: planar_offset_distance(distance.millimetres())?,
                }
            }
            FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile }) => {
                let compiled_profile = match self
                    .snapshot
                    .feature(*profile)
                    .map(|feature| feature.kind())
                {
                    Some(FeatureKind::Sketch(sketch)) => {
                        let regions = sketch.solved_regions().map_err(|error| {
                            ExactBRepGraphError::UnsolvedProfile(*profile, error)
                        })?;
                        let [region] = regions.as_slice() else {
                            return Err(ExactBRepGraphError::UnsupportedProfile(*profile));
                        };
                        self.compile_sketch_profile(
                            *profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    _ => self.compile_profile(*profile, None, identity_frame())?,
                };
                ExactBRepOperation::PlanarSurface {
                    profile: compiled_profile,
                }
            }
            FeatureKind::SurfaceTrim { target, cutter } => ExactBRepOperation::SurfaceTrim {
                target: self.body_id(*target)?,
                cutter: self.body_id(*cutter)?,
            },
            FeatureKind::SurfaceExtend { target, distance } => ExactBRepOperation::SurfaceExtend {
                target: self.body_id(*target)?,
                distance_bits: positive_distance(
                    distance.millimetres(),
                    self.snapshot.tolerance().linear_mm(),
                )?,
            },
            FeatureKind::SurfaceKnit {
                surfaces,
                tolerance,
                make_solid,
            } => ExactBRepOperation::SurfaceKnit {
                surfaces: surfaces
                    .iter()
                    .map(|surface| self.body_id(*surface))
                    .collect::<Result<_, _>>()?,
                tolerance_bits: tolerance.millimetres().to_bits(),
                make_solid: *make_solid,
            },
            FeatureKind::SurfaceThicken {
                target,
                thickness,
                direction,
            } => ExactBRepOperation::SurfaceThicken {
                target: self.body_id(*target)?,
                thickness_bits: positive_distance(
                    thickness.millimetres(),
                    self.snapshot.tolerance().linear_mm(),
                )?,
                direction: (*direction).into(),
            },
            FeatureKind::Sweep { profile, path, up } => {
                let compiled_profile = match self
                    .snapshot
                    .feature(*profile)
                    .map(|feature| feature.kind())
                {
                    Some(FeatureKind::Sketch(sketch)) => {
                        let regions = sketch.solved_regions().map_err(|error| {
                            ExactBRepGraphError::UnsolvedProfile(*profile, error)
                        })?;
                        let [region] = regions.as_slice() else {
                            return Err(ExactBRepGraphError::UnsupportedProfile(*profile));
                        };
                        self.compile_sketch_profile(
                            *profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    _ => self.compile_profile(*profile, None, identity_frame())?,
                };
                self.compile_sweep(compiled_profile, *path, *up)?
            }
            FeatureKind::WeldmentMember(spec) => {
                let compiled_profile = match self
                    .snapshot
                    .feature(spec.profile)
                    .map(|feature| feature.kind())
                {
                    Some(FeatureKind::Sketch(sketch)) => {
                        let regions = sketch.solved_regions().map_err(|error| {
                            ExactBRepGraphError::UnsolvedProfile(spec.profile, error)
                        })?;
                        let [region] = regions.as_slice() else {
                            return Err(ExactBRepGraphError::UnsupportedProfile(spec.profile));
                        };
                        self.compile_sketch_profile(
                            spec.profile,
                            region.id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    _ => self.compile_profile(spec.profile, None, identity_frame())?,
                };
                let profile = self.compile_oriented_profile(
                    compiled_profile,
                    feature_id,
                    spec.orientation_degrees,
                )?;
                let Some(FeatureKind::SpatialPath { segments }) = self
                    .snapshot
                    .feature(spec.path)
                    .map(|feature| feature.kind())
                else {
                    return Err(ExactBRepGraphError::UnsupportedFeature(spec.path));
                };
                ExactBRepOperation::SpatialSweep {
                    profile,
                    path: spatial_path(spec.path, segments, self.snapshot.tolerance().linear_mm())?,
                    up_bits: None,
                }
            }
            FeatureKind::WeldmentJoint(spec) => {
                let member_line = |member_id: FeatureId| {
                    let Some(FeatureKind::WeldmentMember(member)) = self
                        .snapshot
                        .feature(member_id)
                        .map(|feature| feature.kind())
                    else {
                        return Err(ExactBRepGraphError::UnsupportedFeature(member_id));
                    };
                    let Some(FeatureKind::SpatialPath { segments }) = self
                        .snapshot
                        .feature(member.path)
                        .map(|feature| feature.kind())
                    else {
                        return Err(ExactBRepGraphError::UnsupportedFeature(member.path));
                    };
                    let [SpatialPathSegment::Line { start_mm, end_mm }] = segments.as_slice()
                    else {
                        return Err(ExactBRepGraphError::InvalidParameter);
                    };
                    Ok((*start_mm, *end_mm))
                };
                let (first_start, first_end) = member_line(spec.first_member)?;
                let (second_start, second_end) = member_line(spec.second_member)?;
                let distance = |left: [f64; 3], right: [f64; 3]| {
                    ((left[0] - right[0]).powi(2)
                        + (left[1] - right[1]).powi(2)
                        + (left[2] - right[2]).powi(2))
                    .sqrt()
                };
                let endpoint_pairs = [
                    (first_start, first_end, second_start, second_end),
                    (first_start, first_end, second_end, second_start),
                    (first_end, first_start, second_start, second_end),
                    (first_end, first_start, second_end, second_start),
                ];
                let Some((joint, first_far, _, second_far)) =
                    endpoint_pairs
                        .into_iter()
                        .find(|(first_joint, _, second_joint, _)| {
                            distance(*first_joint, *second_joint) <= ROUNDING
                        })
                else {
                    return Err(ExactBRepGraphError::InvalidParameter);
                };
                let direction = |far: [f64; 3]| {
                    let length = distance(joint, far);
                    [
                        (far[0] - joint[0]) / length,
                        (far[1] - joint[1]) / length,
                        (far[2] - joint[2]) / length,
                    ]
                };
                ExactBRepOperation::WeldmentJoint {
                    first: self.body_id(spec.first_member)?,
                    second: self.body_id(spec.second_member)?,
                    joint_point_bits: joint.map(f64::to_bits),
                    first_direction_bits: direction(first_far).map(f64::to_bits),
                    second_direction_bits: direction(second_far).map(f64::to_bits),
                    policy: match spec.policy {
                        WeldmentJointPolicy::Butt => ExactBRepWeldmentJointPolicy::Butt,
                        WeldmentJointPolicy::Miter => ExactBRepWeldmentJointPolicy::Miter,
                    },
                    primary: match spec.primary {
                        WeldmentJointPrimary::First => ExactBRepWeldmentJointPrimary::First,
                        WeldmentJointPrimary::Second => ExactBRepWeldmentJointPrimary::Second,
                    },
                }
            }
            FeatureKind::Loft {
                sections,
                guide,
                continuity,
            } => ExactBRepOperation::Loft {
                sections: self.compile_loft_sections(sections)?,
                guide: guide
                    .map(
                        |guide| match self.snapshot.feature(guide).map(|feature| feature.kind()) {
                            Some(FeatureKind::SpatialPath { segments }) => {
                                spatial_path(guide, segments, self.snapshot.tolerance().linear_mm())
                            }
                            _ => Err(ExactBRepGraphError::UnsupportedFeature(guide)),
                        },
                    )
                    .transpose()?,
                continuity: (*continuity).into(),
            },
            FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
                sections,
                guide,
                continuity,
            }) => ExactBRepOperation::LoftSurface {
                sections: self.compile_loft_sections(sections)?,
                guide: guide
                    .map(
                        |guide| match self.snapshot.feature(guide).map(|feature| feature.kind()) {
                            Some(FeatureKind::SpatialPath { segments }) => {
                                spatial_path(guide, segments, self.snapshot.tolerance().linear_mm())
                            }
                            _ => Err(ExactBRepGraphError::UnsupportedFeature(guide)),
                        },
                    )
                    .transpose()?,
                continuity: (*continuity).into(),
            },
            FeatureKind::SheetMetal(spec) => ExactBRepOperation::SheetMetal {
                base_mm_bits: spec
                    .base_mm
                    .iter()
                    .map(|corner| corner.map(canonical_bits))
                    .collect(),
                thickness_bits: positive_distance(
                    spec.thickness.millimetres(),
                    self.snapshot.tolerance().linear_mm(),
                )?,
                k_factor_bits: canonical_bits(spec.k_factor),
                bends: spec
                    .bends
                    .iter()
                    .map(|bend| {
                        Ok(ExactBRepSheetMetalBend {
                            parent: bend.parent,
                            edge: bend.edge,
                            length_bits: positive_distance(
                                bend.length.millimetres(),
                                self.snapshot.tolerance().linear_mm(),
                            )?,
                            angle_degrees_bits: canonical_bits(bend.angle_degrees),
                            inner_radius_bits: positive_distance(
                                bend.inner_radius.millimetres(),
                                self.snapshot.tolerance().linear_mm(),
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, ExactBRepGraphError>>()?,
            },
            FeatureKind::ImportedExactBody(spec) => ExactBRepOperation::ImportedExact {
                source_sha256: spec.source_sha256,
                source_byte_len: spec.source_byte_len,
                result_fingerprint: spec.result_fingerprint.clone(),
            },
            FeatureKind::RigidTransform { target, transform } => {
                ExactBRepOperation::RigidTransform {
                    target: self.body_id(*target)?,
                    matrix_bits: transform.matrix().map(f64::to_bits),
                }
            }
            _ => return Err(ExactBRepGraphError::UnsupportedFeature(feature_id)),
        };
        let bounds = match feature.kind() {
            FeatureKind::ImportedExactBody(spec) if valid_bounds(spec.bounds_mm) => {
                Some(spec.bounds_mm)
            }
            FeatureKind::ImportedExactBody(_) => {
                return Err(ExactBRepGraphError::UnresolvedExtent);
            }
            _ => operation_bounds(
                &operation,
                &self.profiles,
                &self.node_bounds,
                self.snapshot.tolerance().linear_mm(),
            )?,
        };
        let id = ExactBRepNodeId(
            self.nodes
                .len()
                .try_into()
                .map_err(|_: std::num::TryFromIntError| ExactBRepGraphError::ResourceLimit)?,
        );
        if self.nodes.len() >= MAX_EXACT_BREP_GRAPH_NODES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        self.nodes.push(ExactBRepNode {
            id,
            source_feature_id: feature_id.0,
            operation,
        });
        self.node_bounds.push(bounds);
        self.visiting.remove(&feature_id);
        self.compiled_bodies.insert(feature_id, id);
        Ok(id)
    }

    pub(super) fn compile_loft_sections(
        &mut self,
        sections: &[LoftSection],
    ) -> Result<Vec<ExactBRepLoftSection>, ExactBRepGraphError> {
        if !(2..=MAX_EXACT_BREP_LOFT_SECTIONS).contains(&sections.len()) {
            return Err(ExactBRepGraphError::InvalidParameter);
        }
        sections
            .iter()
            .map(|section| {
                let sketch_region = self
                    .snapshot
                    .feature(section.profile)
                    .and_then(|feature| match feature.kind() {
                        FeatureKind::Sketch(sketch) => sketch.solved_regions().ok(),
                        _ => None,
                    })
                    .and_then(|regions| <[_; 1]>::try_from(regions).ok())
                    .map(|[region]| region.id);
                let profile = match sketch_region {
                    Some(region_id) => {
                        self.compile_sketch_profile(
                            section.profile,
                            region_id,
                            FeatureDirection::AlongNormal,
                        )?
                        .0
                    }
                    None => self.compile_profile(section.profile, None, identity_frame())?,
                };
                Ok(ExactBRepLoftSection {
                    profile,
                    elevation_bits: finite_coordinate(section.elevation_mm)?.to_bits(),
                })
            })
            .collect()
    }

    /// The profile a pad sweeps, the origin of its plane and the sweep direction. A plain
    /// profile lies in the XY plane; a sketch named without a region must have one.
    pub(super) fn compile_pad_profile(
        &mut self,
        profile: PadProfile,
        direction: FeatureDirection,
    ) -> Result<(ExactBRepProfileId, [f64; 3], [f64; 3]), ExactBRepGraphError> {
        let feature_id = profile.feature_id();
        let region_id = match profile {
            PadProfile::SketchRegion { region, .. } => Some(region),
            PadProfile::Feature(_) => match self.snapshot.feature(feature_id).map(|f| f.kind()) {
                Some(FeatureKind::Sketch(sketch)) => {
                    let regions = sketch
                        .solved_regions()
                        .map_err(|error| ExactBRepGraphError::UnsolvedProfile(feature_id, error))?;
                    let [region] = regions.as_slice() else {
                        return Err(ExactBRepGraphError::UnsupportedProfile(feature_id));
                    };
                    Some(region.id)
                }
                _ => None,
            },
        };
        if let Some(region_id) = region_id {
            return self.compile_sketch_profile(feature_id, region_id, direction);
        }
        let plane = WorkplaneFrame::principal(ketchup_geometry::sketch::PrincipalPlane::Xy);
        let direction = direction
            .vector(plane.normal)
            .ok_or(ExactBRepGraphError::InvalidParameter)?;
        Ok((
            self.compile_profile(feature_id, None, frame_bits(plane, direction))?,
            plane.origin_mm,
            direction,
        ))
    }

    pub(super) fn compile_sketch_profile(
        &mut self,
        sketch_id: FeatureId,
        region_id: SketchRegionId,
        direction: FeatureDirection,
    ) -> Result<(ExactBRepProfileId, [f64; 3], [f64; 3]), ExactBRepGraphError> {
        let sketch = self
            .snapshot
            .feature(sketch_id)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Sketch(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let workplane = self
            .snapshot
            .feature(sketch.workplane)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Workplane(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let direction = direction
            .vector(workplane.frame.normal)
            .ok_or(ExactBRepGraphError::InvalidParameter)?;
        let frame = frame_bits(workplane.frame, direction);
        Ok((
            self.compile_profile(sketch_id, Some(region_id), frame)?,
            workplane.frame.origin_mm,
            direction,
        ))
    }

    pub(super) fn compile_sketch_spatial_path(
        &mut self,
        sketch_id: FeatureId,
    ) -> Result<ExactBRepSpatialPath, ExactBRepGraphError> {
        let sketch = self
            .snapshot
            .feature(sketch_id)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Sketch(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let workplane = self
            .snapshot
            .feature(sketch.workplane)
            .and_then(|feature| match feature.kind() {
                FeatureKind::Workplane(spec) => Some(spec),
                _ => None,
            })
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let segments = solved_sketch_sweep_path(sketch, self.snapshot.tolerance().linear_mm())
            .ok_or(ExactBRepGraphError::UnsupportedProfile(sketch_id))?;
        let to_world = |point: [f64; 2]| {
            [0, 1, 2].map(|axis| {
                workplane.frame.origin_mm[axis]
                    + workplane.frame.x_axis[axis] * point[0]
                    + workplane.frame.y_axis[axis] * point[1]
            })
        };
        let segments = segments
            .into_iter()
            .map(|segment| match segment {
                ProfileSegment::Spline { .. } => {
                    Err(ExactBRepGraphError::UnsupportedProfile(sketch_id))
                }
                ProfileSegment::Line { start_mm, end_mm } => Ok(SpatialPathSegment::Line {
                    start_mm: to_world(start_mm),
                    end_mm: to_world(end_mm),
                }),
                ProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => Ok(SpatialPathSegment::CircularArc {
                    start_mm: to_world(start_mm),
                    end_mm: to_world(end_mm),
                    center_mm: to_world(center_mm),
                    normal: workplane.frame.normal,
                    clockwise,
                }),
                ProfileSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => Ok(SpatialPathSegment::CubicBezier {
                    start_mm: to_world(start_mm),
                    control_1_mm: to_world(control_1_mm),
                    control_2_mm: to_world(control_2_mm),
                    end_mm: to_world(end_mm),
                }),
            })
            .collect::<Result<Vec<_>, _>>()?;
        spatial_path(sketch_id, &segments, self.snapshot.tolerance().linear_mm())
    }

    pub(super) fn resolve_extent(
        &self,
        origin_mm: [f64; 3],
        direction: [f64; 3],
        extent: &FeatureExtent,
        target_bounds: Option<[[f64; 3]; 2]>,
    ) -> Result<ExactBRepLinearInterval, ExactBRepGraphError> {
        match extent {
            FeatureExtent::Blind(distance) => {
                let distance = distance.millimetres();
                linear_interval(
                    direction,
                    distance.min(0.0),
                    distance.max(0.0),
                    self.snapshot.tolerance().linear_mm(),
                )
            }
            FeatureExtent::ThroughAll => through_all_interval(
                origin_mm,
                direction,
                target_bounds.ok_or(ExactBRepGraphError::UnresolvedExtent)?,
                self.snapshot.tolerance().linear_mm(),
            ),
            FeatureExtent::UpToFace(reference) => linear_interval(
                direction,
                0.0,
                self.resolve_face_distance(origin_mm, direction, reference)?,
                self.snapshot.tolerance().linear_mm(),
            ),
            FeatureExtent::Symmetric(distance) => {
                let half = distance.millimetres() * 0.5;
                linear_interval(
                    direction,
                    -half,
                    half,
                    self.snapshot.tolerance().linear_mm(),
                )
            }
            FeatureExtent::Bidirectional { along, opposite } => {
                let along = self.resolve_extent_end(origin_mm, direction, along, target_bounds)?;
                let opposite = self.resolve_extent_end(
                    origin_mm,
                    direction.map(|component| -component),
                    opposite,
                    target_bounds,
                )?;
                linear_interval(
                    direction,
                    -opposite,
                    along,
                    self.snapshot.tolerance().linear_mm(),
                )
            }
        }
    }

    pub(super) fn resolve_extent_end(
        &self,
        origin_mm: [f64; 3],
        direction: [f64; 3],
        end: &FeatureExtentEnd,
        target_bounds: Option<[[f64; 3]; 2]>,
    ) -> Result<f64, ExactBRepGraphError> {
        match end {
            FeatureExtentEnd::Blind(distance) => Ok(distance.millimetres()),
            FeatureExtentEnd::ThroughAll => through_all_distance(
                origin_mm,
                direction,
                target_bounds.ok_or(ExactBRepGraphError::UnresolvedExtent)?,
                self.snapshot.tolerance().linear_mm(),
            ),
            FeatureExtentEnd::UpToFace(reference) => {
                self.resolve_face_distance(origin_mm, direction, reference)
            }
        }
    }

    pub(super) fn resolve_face_distance(
        &self,
        origin_mm: [f64; 3],
        direction: [f64; 3],
        reference: &crate::exact_product::BodySubshapeRef,
    ) -> Result<f64, ExactBRepGraphError> {
        let frame = self
            .snapshot
            .resolved_planar_face_workplane_frame(reference)
            .ok_or(ExactBRepGraphError::UnresolvedExtent)?;
        let denominator = dot(direction, frame.normal);
        if denominator.abs() <= self.snapshot.tolerance().linear_mm() {
            return Err(ExactBRepGraphError::AmbiguousExtent);
        }
        let distance = dot(sub(frame.origin_mm, origin_mm), frame.normal) / denominator;
        if !distance.is_finite()
            || distance <= self.snapshot.tolerance().linear_mm()
            || distance > MAX_ABS_MM
        {
            return Err(ExactBRepGraphError::UnresolvedExtent);
        }
        let face_graph = ExactBRepGraph::from_snapshot(
            self.snapshot,
            reference.definition_id,
            reference.producer_feature_id,
        )?;
        if !face_graph.names_durable_reference(reference) {
            return Err(ExactBRepGraphError::UnresolvedExtent);
        }
        let intersection = [0, 1, 2].map(|axis| origin_mm[axis] + direction[axis] * distance);
        let bounds = face_graph
            .producer_bounds_mm()?
            .ok_or(ExactBRepGraphError::UnresolvedExtent)?;
        let tolerance = self.snapshot.tolerance().linear_mm();
        if (0..3).any(|axis| {
            intersection[axis] < bounds[0][axis] - tolerance
                || intersection[axis] > bounds[1][axis] + tolerance
        }) {
            return Err(ExactBRepGraphError::UnresolvedExtent);
        }
        Ok(distance)
    }

    pub(super) fn compile_profile(
        &mut self,
        feature_id: FeatureId,
        region_id: Option<SketchRegionId>,
        frame_bits: [u64; 12],
    ) -> Result<ExactBRepProfileId, ExactBRepGraphError> {
        let key = (feature_id, region_id, frame_bits);
        if let Some(id) = self.profile_ids.get(&key) {
            return Ok(*id);
        }
        if self.profiles.len() >= MAX_EXACT_BREP_GRAPH_PROFILES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        let feature = self
            .snapshot
            .feature(feature_id)
            .filter(|feature| feature.definition_id() == self.definition_id)
            .ok_or(ExactBRepGraphError::FeatureNotFound(feature_id))?;
        if self.snapshot.feature_is_suppressed(feature_id) {
            return Err(ExactBRepGraphError::SuppressedFeature(feature_id));
        }
        let (geometry, segment_entity_ids) = match (feature.kind(), region_id) {
            (FeatureKind::Profile { segments, closed }, None) => {
                // The exact kernel builds a spline only as a whole closed curve.
                if segments.len() > 1
                    && segments
                        .iter()
                        .any(|segment| matches!(segment, ProfileSegment::Spline { .. }))
                {
                    return Err(ExactBRepGraphError::UnsupportedProfile(feature_id));
                }
                let geometry =
                    boundary_geometry(segments, *closed, self.snapshot.tolerance().linear_mm())?;
                // A spline is one unnamed curve; boundary segments are named by position.
                let segment_count = match geometry {
                    ExactBRepPlanarGeometry::Spline { .. } => 0,
                    _ => segments.len(),
                };
                (
                    geometry,
                    (1..=segment_count)
                        .map(|index| {
                            u64::try_from(index).map_err(|_: std::num::TryFromIntError| {
                                ExactBRepGraphError::ResourceLimit
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            (FeatureKind::Sketch(sketch), Some(region_id)) => {
                let region = sketch
                    .solved_regions()
                    .map_err(|error| ExactBRepGraphError::UnsolvedProfile(feature_id, error))?
                    .into_iter()
                    .find(|region| region.id == region_id)
                    .ok_or(ExactBRepGraphError::UnsupportedProfile(feature_id))?;
                // Face names follow boundary order; a region with holes stays unnamed.
                let segment_entity_ids = if region.holes.is_empty() {
                    region.outer_entity_ids.iter().map(|id| id.0).collect()
                } else {
                    Vec::new()
                };
                (
                    solved_geometry(&region, self.snapshot.tolerance().linear_mm())?,
                    segment_entity_ids,
                )
            }
            (FeatureKind::Sketch(sketch), None) => (
                boundary_geometry(
                    &solved_sketch_sweep_path(sketch, self.snapshot.tolerance().linear_mm())
                        .ok_or(ExactBRepGraphError::UnsupportedProfile(feature_id))?,
                    false,
                    self.snapshot.tolerance().linear_mm(),
                )?,
                Vec::new(),
            ),
            _ => return Err(ExactBRepGraphError::UnsupportedProfile(feature_id)),
        };
        let id = ExactBRepProfileId(
            self.profiles
                .len()
                .try_into()
                .map_err(|_: std::num::TryFromIntError| ExactBRepGraphError::ResourceLimit)?,
        );
        self.profiles.push(ExactBRepProfile {
            id,
            source_feature_id: feature_id.0,
            region_id: region_id.map(|id| id.0),
            frame_bits,
            segment_entity_ids,
            geometry,
        });
        self.profile_ids.insert(key, id);
        Ok(id)
    }

    pub(super) fn compile_oriented_profile(
        &mut self,
        source: ExactBRepProfileId,
        member_feature_id: FeatureId,
        orientation_degrees: f64,
    ) -> Result<ExactBRepProfileId, ExactBRepGraphError> {
        if self.profiles.len() >= MAX_EXACT_BREP_GRAPH_PROFILES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        let mut profile = self
            .profiles
            .get(source.0 as usize)
            .cloned()
            .ok_or(ExactBRepGraphError::InvalidParameter)?;
        let id = ExactBRepProfileId(
            self.profiles
                .len()
                .try_into()
                .map_err(|_: std::num::TryFromIntError| ExactBRepGraphError::ResourceLimit)?,
        );
        profile.id = id;
        profile.source_feature_id = member_feature_id.0;
        rotate_planar_geometry(&mut profile.geometry, orientation_degrees)?;
        self.profiles.push(profile);
        Ok(id)
    }

    /// The spatial sweep lays its profile out in a section frame at the path
    /// start. A sketched profile already sits in the world, in a plane through
    /// the path start facing along it: re-express it in that section frame so
    /// the swept body starts exactly where it was sketched.
    /// A sweep of `profile` along `path`: along a spatial path (or a sketch
    /// taken as one) the profile is framed at the path start, optionally on a
    /// fixed `up`; a planar path keeps it square to its plane by itself.
    fn compile_sweep(
        &mut self,
        profile: ExactBRepProfileId,
        path: FeatureId,
        up: Option<[f64; 3]>,
    ) -> Result<ExactBRepOperation, ExactBRepGraphError> {
        let spatial = match self.snapshot.feature(path).map(|feature| feature.kind()) {
            Some(FeatureKind::SpatialPath { segments }) => Some(spatial_path(
                path,
                segments,
                self.snapshot.tolerance().linear_mm(),
            )?),
            Some(FeatureKind::Sketch(_)) => Some(self.compile_sketch_spatial_path(path)?),
            _ => None,
        };
        let up = up
            .map(|up| normalize_within(up, ROUNDING).ok_or(ExactBRepGraphError::InvalidParameter))
            .transpose()?;
        Ok(match (spatial, up) {
            (Some(spatial), up) => ExactBRepOperation::SpatialSweep {
                profile: self.compile_section_profile(profile, &spatial, up)?,
                path: spatial,
                up_bits: up.map(|up| up.map(f64::to_bits)),
            },
            (None, Some(_)) => return Err(ExactBRepGraphError::UnsupportedFeature(path)),
            (None, None) => ExactBRepOperation::Sweep {
                profile,
                path: self.compile_profile(path, None, identity_frame())?,
            },
        })
    }

    pub(super) fn compile_section_profile(
        &mut self,
        source: ExactBRepProfileId,
        path: &ExactBRepSpatialPath,
        up: Option<[f64; 3]>,
    ) -> Result<ExactBRepProfileId, ExactBRepGraphError> {
        let mut profile = self
            .profiles
            .get(source.0 as usize)
            .cloned()
            .ok_or(ExactBRepGraphError::InvalidParameter)?;
        if profile.frame_bits == identity_frame() {
            return Ok(source);
        }
        if self.profiles.len() >= MAX_EXACT_BREP_GRAPH_PROFILES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        let (start, tangent) = match path.segments.first() {
            Some(ExactBRepSpatialPathSegment::Line {
                start_bits,
                end_bits,
            }) => {
                let start = start_bits.map(f64::from_bits);
                (start, sub(end_bits.map(f64::from_bits), start))
            }
            Some(ExactBRepSpatialPathSegment::CircularArc {
                start_bits,
                center_bits,
                normal_bits,
                clockwise,
                ..
            }) => {
                let start = start_bits.map(f64::from_bits);
                let tangent = cross(
                    normal_bits.map(f64::from_bits),
                    sub(start, center_bits.map(f64::from_bits)),
                );
                (
                    start,
                    tangent.map(|value| if *clockwise { -value } else { value }),
                )
            }
            Some(ExactBRepSpatialPathSegment::CubicBezier {
                start_bits,
                control_1_bits,
                ..
            }) => {
                let start = start_bits.map(f64::from_bits);
                (start, sub(control_1_bits.map(f64::from_bits), start))
            }
            None => return Err(ExactBRepGraphError::InvalidParameter),
        };
        let unit = |vector: [f64; 3]| {
            let length = dot(vector, vector).sqrt();
            vector.map(|value| value / length)
        };
        let tangent = unit(tangent);
        // The same section frame the exact kernel builds at the path start.
        let reference = if dot(
            cross(tangent, [0.0, 0.0, 1.0]),
            cross(tangent, [0.0, 0.0, 1.0]),
        ) <= ROUNDING * ROUNDING
        {
            [0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        };
        let (section_u, section_v) = match up {
            Some(up) => (unit(cross(tangent, up)), unit(up)),
            None => {
                let section_u = unit(cross(tangent, reference));
                (section_u, unit(cross(section_u, tangent)))
            }
        };
        let section_normal = cross(section_u, section_v);
        let [origin, x_axis, y_axis, _] = profile.frame().to_vectors();
        let from_start = sub(origin, start);
        if dot(cross(x_axis, y_axis), section_normal).abs() < 1.0 - ROUNDING
            || dot(from_start, section_normal).abs() > ROUNDING
        {
            return Err(ExactBRepGraphError::InvalidParameter);
        }
        map_planar_geometry(
            &mut profile.geometry,
            PlanarRigidMap {
                linear: [
                    [dot(x_axis, section_u), dot(y_axis, section_u)],
                    [dot(x_axis, section_v), dot(y_axis, section_v)],
                ],
                offset: [dot(from_start, section_u), dot(from_start, section_v)],
            },
        )?;
        let id = ExactBRepProfileId(
            self.profiles
                .len()
                .try_into()
                .map_err(|_: std::num::TryFromIntError| ExactBRepGraphError::ResourceLimit)?,
        );
        profile.id = id;
        profile.frame_bits = identity_frame();
        self.profiles.push(profile);
        Ok(id)
    }
}
