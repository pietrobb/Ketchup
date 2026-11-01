use super::*;
use ketchup_geometry::mass::MassProperties;
use std::collections::HashMap;

/// The values of `pairs` grouped by key, each group sorted: one pass over the
/// adjacency evidence instead of one per face or edge.
fn grouped(pairs: impl IntoIterator<Item = (u32, u32)>) -> HashMap<u32, Vec<u32>> {
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for (key, value) in pairs {
        groups.entry(key).or_default().push(value);
    }
    for values in groups.values_mut() {
        values.sort_unstable();
    }
    groups
}

pub(super) fn collect_output(
    native: cxx::UniquePtr<ffi::NativeOperationResult>,
    operation: &'static str,
    input: &str,
    history_confidence: HistoryConfidence,
) -> Result<ExactOpOutput, GeometryError> {
    let input_digest = stable_digest(input);
    let native_ref = native.as_ref().ok_or_else(|| GeometryError {
        code: GeometryErrorCode::NullResult,
        diagnostic: "Native facade returned no result object".to_owned(),
        operation,
        input_digest: input_digest.clone(),
        backend_fingerprint: BACKEND_FINGERPRINT,
    })?;
    let status = native_ref.status_code();
    let diagnostic = native_ref.diagnostic();
    if status != 0 || !native_ref.valid() {
        return Err(GeometryError {
            code: native_status(status),
            diagnostic,
            operation,
            input_digest,
            backend_fingerprint: BACKEND_FINGERPRINT,
        });
    }

    let summary = native_ref.topology_summary();
    let edges_of_face = grouped(
        native_ref
            .face_edge_evidence()
            .into_iter()
            .map(|entry| (entry.face_ordinal, entry.edge_ordinal)),
    );
    let faces_of_edge = grouped(
        native_ref
            .edge_face_evidence()
            .into_iter()
            .map(|entry| (entry.edge_ordinal, entry.face_ordinal)),
    );
    let faces = native_ref
        .face_evidence()
        .into_iter()
        .map(|face| {
            let axis_origin_mm = face.has_axis.then_some(Point3 {
                x: face.axis_origin_x,
                y: face.axis_origin_y,
                z: face.axis_origin_z,
            });
            let axis_direction = face.has_axis.then_some(Point3 {
                x: face.axis_direction_x,
                y: face.axis_direction_y,
                z: face.axis_direction_z,
            });
            let signature = if face.has_axis {
                format!(
                    "axis-v2:{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                    face.ordinal,
                    face.surface_kind,
                    face.area_mm2.to_bits(),
                    face.centroid_x.to_bits(),
                    face.centroid_y.to_bits(),
                    face.centroid_z.to_bits(),
                    face.normal_x.to_bits(),
                    face.normal_y.to_bits(),
                    face.normal_z.to_bits(),
                    face.axis_origin_x.to_bits(),
                    face.axis_origin_y.to_bits(),
                    face.axis_origin_z.to_bits(),
                    face.axis_direction_x.to_bits(),
                    face.axis_direction_y.to_bits(),
                    face.axis_direction_z.to_bits(),
                    face.edge_count
                )
            } else {
                format!(
                    "{}:{}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{:016x}:{}",
                    face.ordinal,
                    face.surface_kind,
                    face.area_mm2.to_bits(),
                    face.centroid_x.to_bits(),
                    face.centroid_y.to_bits(),
                    face.centroid_z.to_bits(),
                    face.normal_x.to_bits(),
                    face.normal_y.to_bits(),
                    face.edge_count
                )
            };
            FaceEvidence {
                ordinal: face.ordinal,
                surface_kind: face.surface_kind,
                area_mm2: face.area_mm2,
                centroid_mm: Point3 {
                    x: face.centroid_x,
                    y: face.centroid_y,
                    z: face.centroid_z,
                },
                normal: Point3 {
                    x: face.normal_x,
                    y: face.normal_y,
                    z: face.normal_z,
                },
                axis_origin_mm,
                axis_direction,
                bounds_mm: Bounds3 {
                    min: Point3 {
                        x: face.min_x,
                        y: face.min_y,
                        z: face.min_z,
                    },
                    max: Point3 {
                        x: face.max_x,
                        y: face.max_y,
                        z: face.max_z,
                    },
                },
                edge_count: face.edge_count,
                edge_ordinals: edges_of_face
                    .get(&face.ordinal)
                    .cloned()
                    .unwrap_or_default(),
                geometric_fingerprint: stable_digest(&signature),
                cone: face.has_cone.then_some(ConeEvidence {
                    axis_direction: Point3 {
                        x: face.cone_axis_x,
                        y: face.cone_axis_y,
                        z: face.cone_axis_z,
                    },
                    normal: Point3 {
                        x: face.cone_normal_x,
                        y: face.cone_normal_y,
                        z: face.cone_normal_z,
                    },
                }),
            }
        })
        .collect::<Vec<_>>();
    let topology = TopologyEvidence {
        vertex_count: summary.vertex_count,
        edge_count: summary.edge_count,
        wire_count: summary.wire_count,
        face_count: summary.face_count,
        shell_count: summary.shell_count,
        solid_count: summary.solid_count,
        volume_mm3: summary.volume_mm3,
        mass: summary.has_mass.then_some(MassProperties {
            centroid_mm: [summary.centroid_x, summary.centroid_y, summary.centroid_z],
            inertia_mm5: [
                summary.inertia_xx,
                summary.inertia_yy,
                summary.inertia_zz,
                summary.inertia_xy,
                summary.inertia_xz,
                summary.inertia_yz,
            ],
        }),
        bounds_mm: Bounds3 {
            min: Point3 {
                x: summary.min_x,
                y: summary.min_y,
                z: summary.min_z,
            },
            max: Point3 {
                x: summary.max_x,
                y: summary.max_y,
                z: summary.max_z,
            },
        },
        faces,
        edges: native_ref
            .edge_evidence()
            .into_iter()
            .map(|edge| {
                let adjacent_face_ordinals = faces_of_edge
                    .get(&edge.ordinal)
                    .cloned()
                    .unwrap_or_default();
                let axis_origin_mm = edge.has_axis.then_some(Point3 {
                    x: edge.axis_origin_x,
                    y: edge.axis_origin_y,
                    z: edge.axis_origin_z,
                });
                let axis_direction = edge.has_axis.then_some(Point3 {
                    x: edge.axis_direction_x,
                    y: edge.axis_direction_y,
                    z: edge.axis_direction_z,
                });
                EdgeEvidence {
                    ordinal: edge.ordinal,
                    curve_kind: edge.curve_kind,
                    length_mm: edge.length_mm,
                    centroid_mm: Point3 {
                        x: edge.centroid_x,
                        y: edge.centroid_y,
                        z: edge.centroid_z,
                    },
                    bounds_mm: Bounds3 {
                        min: Point3 {
                            x: edge.min_x,
                            y: edge.min_y,
                            z: edge.min_z,
                        },
                        max: Point3 {
                            x: edge.max_x,
                            y: edge.max_y,
                            z: edge.max_z,
                        },
                    },
                    closed: edge.closed,
                    circle_radius_mm: edge.has_circle.then_some(edge.circle_radius_mm),
                    axis_origin_mm,
                    axis_direction,
                    adjacent_face_ordinals,
                }
            })
            .collect(),
    };
    let edge_geometry_is_valid = topology.edges.iter().all(|edge| {
        let finite_point =
            |point: Point3| [point.x, point.y, point.z].into_iter().all(f64::is_finite);
        let circle_is_coherent = match edge.circle_radius_mm {
            None => edge.curve_kind != "circle",
            Some(radius) => edge.curve_kind == "circle" && radius.is_finite() && radius > 0.0,
        };
        let axis_is_coherent = match (edge.axis_origin_mm, edge.axis_direction) {
            (Some(origin), Some(direction)) => {
                matches!(edge.curve_kind.as_str(), "line" | "circle")
                    && finite_point(origin)
                    && finite_point(direction)
                    && (direction.x * direction.x
                        + direction.y * direction.y
                        + direction.z * direction.z
                        - 1.0)
                        .abs()
                        <= NEGLIGIBLE
            }
            (None, None) => !matches!(edge.curve_kind.as_str(), "line" | "circle"),
            _ => false,
        };
        // A degenerated edge (a cone apex, a sphere pole) is a point.
        let length_is_coherent = if edge.curve_kind == "degenerate" {
            edge.length_mm == 0.0
                && edge.bounds_mm.min == edge.bounds_mm.max
                && edge.centroid_mm == edge.bounds_mm.min
        } else {
            edge.length_mm.is_finite() && edge.length_mm > 0.0
        };
        !edge.curve_kind.is_empty()
            && length_is_coherent
            && finite_point(edge.centroid_mm)
            && finite_point(edge.bounds_mm.min)
            && finite_point(edge.bounds_mm.max)
            && edge.bounds_mm.min.x <= edge.bounds_mm.max.x
            && edge.bounds_mm.min.y <= edge.bounds_mm.max.y
            && edge.bounds_mm.min.z <= edge.bounds_mm.max.z
            && circle_is_coherent
            && axis_is_coherent
    });
    if topology.edges.len() != topology.edge_count as usize || !edge_geometry_is_valid {
        return Err(GeometryError {
            code: GeometryErrorCode::InvalidShape,
            diagnostic: "OCCT returned incomplete or invalid exact edge evidence".to_owned(),
            operation,
            input_digest,
            backend_fingerprint: BACKEND_FINGERPRINT,
        });
    }
    let result_signature = format!(
        "{}:{}:{}:{}:{}:{:016x}:{:?}",
        BACKEND_FINGERPRINT,
        operation,
        input_digest,
        topology.face_count,
        topology.solid_count,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm
    );
    let result_fingerprint = stable_digest(&result_signature);
    let mut history = native_ref
        .history_evidence()
        .into_iter()
        .map(|entry| HistoryEvidence {
            semantic_role: (!entry.semantic_role.is_empty()).then_some(entry.semantic_role),
            relation: entry.relation,
            source_element_id: entry.source_element_id,
            output_face_ordinal: entry.output_present.then_some(entry.output_ordinal),
            output_edge_ordinal: None,
        })
        .collect::<Vec<_>>();
    history.extend(
        native_ref
            .edge_history_evidence()
            .into_iter()
            .map(|entry| HistoryEvidence {
                semantic_role: (!entry.semantic_role.is_empty()).then_some(entry.semantic_role),
                relation: entry.relation,
                source_element_id: entry.source_element_id,
                output_face_ordinal: None,
                output_edge_ordinal: entry.output_present.then_some(entry.output_ordinal),
            }),
    );
    let rejected_shapes = native_ref
        .rejected_shapes()
        .into_iter()
        .map(|shape| RejectedShape {
            ordinal: shape.ordinal,
            kind: shape.kind,
            source_entity: (shape.source_entity != 0).then_some(shape.source_entity),
            reason: shape.reason,
        })
        .collect();

    Ok(ExactOpOutput {
        body: ExactBody {
            native,
            result_fingerprint,
            topology,
        },
        topology_history: history,
        tolerance_report: ToleranceReport {
            profile: TOLERANCE_PROFILE,
            shape_valid: true,
            accepted_exact_solid: summary.solid_count > 0,
        },
        diagnostics: vec![GeometryDiagnostic {
            code: if summary.solid_count > 0 {
                "valid_exact_solid"
            } else {
                "valid_exact_planar_face"
            },
            message: diagnostic,
        }],
        input_digest,
        backend_fingerprint: BACKEND_FINGERPRINT,
        history_confidence,
        rejected_shapes,
    })
}
