use ketchup_exact::{
    BoxSpec, CircleExtrudeSpec, ExactBackend, ExactOpOutput, ExactVolumeMeshOptions,
    GeometryErrorCode, PlanarLoftSection, PlanarLoftSpec, PlanarProfileLoop, PlanarProfileSegment,
    Point3, Size3, SplineLoftSection, SplineLoftSpec,
};

const COORDINATE_LIMIT_MM: f64 = 1_000_000.0;

fn assert_close(actual: f64, expected: f64) {
    let absolute = (actual - expected).abs();
    assert!(
        absolute <= 1.0e-6 || absolute <= expected.abs() * 1.0e-10,
        "{actual} != {expected}"
    );
}

fn assert_valid(output: &ExactOpOutput) {
    assert!(output.tolerance_report.shape_valid);
    assert!(output.tolerance_report.accepted_exact_solid);
    assert_eq!(output.body.topology.solid_count, 1);
    assert!(output.body.topology.volume_mm3.is_finite());
    assert!(output.body.topology.volume_mm3 > 0.0);
}

fn reverse_planar_loop(planar_loop: &PlanarProfileLoop) -> PlanarProfileLoop {
    let PlanarProfileLoop::Segments(segments) = planar_loop else {
        return planar_loop.clone();
    };
    PlanarProfileLoop::Segments(
        segments
            .iter()
            .rev()
            .map(|segment| match *segment {
                PlanarProfileSegment::Line { start_mm, end_mm } => PlanarProfileSegment::Line {
                    start_mm: end_mm,
                    end_mm: start_mm,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => PlanarProfileSegment::CircularArc {
                    start_mm: end_mm,
                    end_mm: start_mm,
                    center_mm,
                    clockwise: !clockwise,
                },
                PlanarProfileSegment::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => PlanarProfileSegment::CubicBezier {
                    start_mm: end_mm,
                    control_1_mm: control_2_mm,
                    control_2_mm: control_1_mm,
                    end_mm: start_mm,
                },
            })
            .collect(),
    )
}

#[test]
fn boxes_may_touch_positive_and_negative_coordinate_limits() {
    let backend = ExactBackend::new();
    let positive = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 999_990.0,
                y: 20.0,
                z: -30.0,
            },
            size_mm: Size3 {
                x: 10.0,
                y: 5.0,
                z: 2.0,
            },
        })
        .expect("a box ending at the positive coordinate limit must succeed");
    let negative = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -COORDINATE_LIMIT_MM,
                y: 20.0,
                z: -30.0,
            },
            size_mm: Size3 {
                x: 10.0,
                y: 5.0,
                z: 2.0,
            },
        })
        .expect("a box beginning at the negative coordinate limit must succeed");

    assert_valid(&positive);
    assert_valid(&negative);
    assert_close(positive.body.topology.bounds_mm.max.x, COORDINATE_LIMIT_MM);
    assert_close(negative.body.topology.bounds_mm.min.x, -COORDINATE_LIMIT_MM);
}

#[test]
fn primitive_box_publishes_complete_unique_face_lineage() {
    let output = ExactBackend::new()
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 37.0,
                y: 23.0,
                z: 19.0,
            },
        })
        .unwrap();
    let roles = output
        .topology_history
        .iter()
        .filter_map(|entry| {
            entry
                .output_face_ordinal
                .map(|_| entry.semantic_role.as_deref())
        })
        .flatten()
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        output.history_confidence,
        ketchup_exact::HistoryConfidence::Complete
    );
    assert_eq!(roles.len(), 6);
    assert_eq!(
        roles,
        std::collections::BTreeSet::from([
            "box.face.back",
            "box.face.bottom",
            "box.face.front",
            "box.face.left",
            "box.face.right",
            "box.face.top",
        ])
    );
    assert!(output.topology_history.iter().all(|entry| {
        entry.output_face_ordinal.is_some()
            && entry.semantic_role.as_deref() == Some(entry.source_element_id.as_str())
            && entry.relation == "generated"
    }));
}

#[test]
fn planar_box_face_offsets_expand_and_contract_exact_volume() {
    let backend = ExactBackend::new();
    let base = backend
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 100.0,
                y: 60.0,
                z: 20.0,
            },
        })
        .unwrap();
    let expanded = backend.offset_body_face(&base.body, 0, 5.0).unwrap();
    let contracted = backend.offset_body_face(&base.body, 0, -5.0).unwrap();

    assert_valid(&expanded);
    assert_valid(&contracted);
    assert!(expanded.body.topology.volume_mm3 > base.body.topology.volume_mm3);
    assert!(contracted.body.topology.volume_mm3 < base.body.topology.volume_mm3);
    assert_eq!(
        backend
            .offset_body_face(&base.body, base.body.topology.face_count, 5.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn maximum_length_may_end_exactly_at_positive_coordinate_limit() {
    let output = ExactBackend::new()
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 900_000.0,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 100_000.0,
                y: 10.0,
                z: 10.0,
            },
        })
        .expect("the maximum length ending at the coordinate limit must succeed");

    assert_valid(&output);
    assert_close(output.body.topology.bounds_mm.min.x, 900_000.0);
    assert_close(output.body.topology.bounds_mm.max.x, COORDINATE_LIMIT_MM);
    assert_close(output.body.topology.volume_mm3, 10_000_000.0);
}

#[test]
fn coordinates_just_outside_the_envelope_are_invalid_parameters() {
    let backend = ExactBackend::new();
    let positive_endpoint = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 999_999.0,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 1.01,
                y: 1.0,
                z: 1.0,
            },
        })
        .expect_err("an endpoint beyond the positive coordinate limit must be rejected");
    let negative_origin = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -1_000_000.01,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
        })
        .expect_err("an origin beyond the negative coordinate limit must be rejected");

    assert_eq!(positive_endpoint.code, GeometryErrorCode::InvalidParameter);
    assert_eq!(negative_origin.code, GeometryErrorCode::InvalidParameter);
}

#[test]
fn non_finite_coordinate_is_rejected() {
    let error = ExactBackend::new()
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: f64::INFINITY,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
        })
        .expect_err("a non-finite coordinate must be rejected");

    assert_eq!(error.code, GeometryErrorCode::NonFiniteParameter);
}

#[test]
fn curved_planar_sweep_is_deterministic_and_fails_closed() {
    let backend = ExactBackend::new();
    let profile = vec![
        PlanarProfileSegment::Line {
            start_mm: [-2.0, -1.0],
            end_mm: [2.0, -1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [2.0, -1.0],
            end_mm: [2.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [2.0, 1.0],
            end_mm: [-2.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-2.0, 1.0],
            end_mm: [-2.0, -1.0],
        },
    ];
    let path = vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [50.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [50.0, 0.0],
            end_mm: [75.0, 25.0],
            center_mm: [50.0, 25.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [75.0, 25.0],
            end_mm: [75.0, 50.0],
        },
    ];
    let output = backend.sweep_planar_profile(&profile, &path).unwrap();
    let repeated = backend.sweep_planar_profile(&profile, &path).unwrap();
    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_close(
        output.body.topology.volume_mm3,
        8.0 * (75.0 + 25.0 * std::f64::consts::FRAC_PI_2),
    );
    assert!(output.body.topology.face_count >= 6);
    assert_eq!(output.body.topology.solid_count, 1);
    assert_eq!(
        output.history_confidence,
        ketchup_exact::HistoryConfidence::Partial
    );
    assert_eq!(output.topology_history.len(), 2);
    assert!(
        output
            .topology_history
            .iter()
            .all(|history| history.output_face_ordinal.is_some())
    );

    let mut sharp_path = path.clone();
    sharp_path[1] = PlanarProfileSegment::Line {
        start_mm: [50.0, 0.0],
        end_mm: [50.0, 25.0],
    };
    assert_eq!(
        backend
            .sweep_planar_profile(&profile, &sharp_path)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    let mut disconnected_path = path;
    disconnected_path[1] = PlanarProfileSegment::CircularArc {
        start_mm: [51.0, 0.0],
        end_mm: [76.0, 25.0],
        center_mm: [51.0, 25.0],
        clockwise: false,
    };
    assert_eq!(
        backend
            .sweep_planar_profile(&profile, &disconnected_path)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );

    let closed_loop = vec![
        PlanarProfileSegment::CircularArc {
            start_mm: [25.0, 0.0],
            end_mm: [-25.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [-25.0, 0.0],
            end_mm: [25.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
    ];
    assert_eq!(
        backend
            .sweep_planar_profile(&profile, &closed_loop)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );

    let overlapping_arcs = vec![
        PlanarProfileSegment::CircularArc {
            start_mm: [25.0, 0.0],
            end_mm: [0.0, -25.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [0.0, -25.0],
            end_mm: [-25.0, 0.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
    ];
    assert_eq!(
        backend
            .sweep_planar_profile(&profile, &overlapping_arcs)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );

    let over_limit = (0..65)
        .map(|index| PlanarProfileSegment::Line {
            start_mm: [index as f64, 0.0],
            end_mm: [index as f64 + 1.0, 0.0],
        })
        .collect::<Vec<_>>();
    assert_eq!(
        backend
            .sweep_planar_profile(&profile, &over_limit)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
}

#[test]
fn non_coplanar_mixed_spatial_sweep_is_exact_deterministic_and_fails_closed() {
    use ketchup_exact::{ExactKernel, SpatialProfileSegment};

    let kernel = ExactKernel::new();
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [-2.0, -1.0],
            end_mm: [2.0, -1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [2.0, -1.0],
            end_mm: [2.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [2.0, 1.0],
            end_mm: [-2.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-2.0, 1.0],
            end_mm: [-2.0, -1.0],
        },
    ];
    let path = [
        SpatialProfileSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [30.0, 0.0, 0.0],
        },
        SpatialProfileSegment::CircularArc {
            start_mm: [30.0, 0.0, 0.0],
            end_mm: [40.0, 10.0, 0.0],
            center_mm: [30.0, 10.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: [40.0, 10.0, 0.0],
            control_1_mm: [40.0, 20.0, 0.0],
            control_2_mm: [40.0, 30.0, 10.0],
            end_mm: [40.0, 40.0, 20.0],
        },
    ];

    let output = kernel.sweep_spatial_profile(&profile, &path).unwrap();
    let repeated = kernel.sweep_spatial_profile(&profile, &path).unwrap();
    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert!(output.body.topology.bounds_mm.max.z > 19.0);
    assert!(output.body.topology.bounds_mm.max.y > 39.0);
    assert_eq!(
        output.history_confidence,
        ketchup_exact::HistoryConfidence::Partial
    );
    assert_eq!(output.topology_history.len(), 2);

    let one_segment = kernel
        .sweep_spatial_profile(&profile, &path[..1])
        .expect("one spatial line segment must produce an exact sweep");
    assert_valid(&one_segment);
    let mut non_finite = path;
    non_finite[2] = SpatialProfileSegment::CubicBezier {
        start_mm: [40.0, 10.0, 0.0],
        control_1_mm: [40.0, 20.0, f64::NAN],
        control_2_mm: [40.0, 30.0, 10.0],
        end_mm: [40.0, 40.0, 20.0],
    };
    assert_eq!(
        kernel
            .sweep_spatial_profile(&profile, &non_finite)
            .unwrap_err()
            .code,
        GeometryErrorCode::NonFiniteParameter
    );
    let mut bad_normal = path;
    bad_normal[1] = SpatialProfileSegment::CircularArc {
        start_mm: [30.0, 0.0, 0.0],
        end_mm: [40.0, 10.0, 0.0],
        center_mm: [30.0, 10.0, 0.0],
        normal: [0.0, 0.0, 2.0],
        clockwise: false,
    };
    assert_eq!(
        kernel
            .sweep_spatial_profile(&profile, &bad_normal)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    let mut off_plane = path;
    off_plane[1] = SpatialProfileSegment::CircularArc {
        start_mm: [30.0, 0.0, 0.0],
        end_mm: [40.0, 10.0, 0.001],
        center_mm: [30.0, 10.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        clockwise: false,
    };
    assert_eq!(
        kernel
            .sweep_spatial_profile(&profile, &off_plane)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    let mut disconnected = path;
    disconnected[2] = SpatialProfileSegment::Line {
        start_mm: [40.0, 11.0, 0.0],
        end_mm: [40.0, 40.0, 0.0],
    };
    assert_eq!(
        kernel
            .sweep_spatial_profile(&profile, &disconnected)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    let mut not_c1 = path;
    not_c1[2] = SpatialProfileSegment::Line {
        start_mm: [40.0, 10.0, 0.0],
        end_mm: [50.0, 10.0, 0.0],
    };
    assert_eq!(
        kernel
            .sweep_spatial_profile(&profile, &not_c1)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
}

#[test]
fn closed_c1_spatial_sweep_produces_one_deterministic_periodic_solid() {
    use ketchup_exact::{ExactKernel, SpatialProfileSegment};

    let kernel = ExactKernel::new();
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [-1.0, -1.0],
            end_mm: [1.0, -1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [1.0, -1.0],
            end_mm: [1.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [1.0, 1.0],
            end_mm: [-1.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-1.0, 1.0],
            end_mm: [-1.0, -1.0],
        },
    ];
    let quarter = |start_mm, end_mm| SpatialProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        clockwise: false,
    };
    let path = [
        quarter([20.0, 0.0, 0.0], [0.0, 20.0, 0.0]),
        quarter([0.0, 20.0, 0.0], [-20.0, 0.0, 0.0]),
        quarter([-20.0, 0.0, 0.0], [0.0, -20.0, 0.0]),
        quarter([0.0, -20.0, 0.0], [20.0, 0.0, 0.0]),
    ];

    let output = kernel.sweep_spatial_profile(&profile, &path).unwrap();
    let repeated = kernel.sweep_spatial_profile(&profile, &path).unwrap();
    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_close(output.body.topology.bounds_mm.min.x, -21.0);
    assert_close(output.body.topology.bounds_mm.max.x, 21.0);
    assert_close(output.body.topology.bounds_mm.min.y, -21.0);
    assert_close(output.body.topology.bounds_mm.max.y, 21.0);
}

#[test]
fn closed_non_planar_spatial_sweep_has_a_periodic_deterministic_frame() {
    use ketchup_exact::{ExactKernel, SpatialProfileSegment};

    let kernel = ExactKernel::new();
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [-2.0, -1.0],
            end_mm: [3.0, -1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [3.0, -1.0],
            end_mm: [-1.0, 2.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-1.0, 2.0],
            end_mm: [-2.0, -1.0],
        },
    ];
    let path = [
        SpatialProfileSegment::CubicBezier {
            start_mm: [30.0, 0.0, 0.0],
            control_1_mm: [30.0, 15.0, 10.0],
            control_2_mm: [15.0, 30.0, 10.0],
            end_mm: [0.0, 30.0, 0.0],
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: [0.0, 30.0, 0.0],
            control_1_mm: [-15.0, 30.0, -10.0],
            control_2_mm: [-30.0, 15.0, -10.0],
            end_mm: [-30.0, 0.0, 0.0],
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: [-30.0, 0.0, 0.0],
            control_1_mm: [-30.0, -15.0, 10.0],
            control_2_mm: [-15.0, -30.0, 10.0],
            end_mm: [0.0, -30.0, 0.0],
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: [0.0, -30.0, 0.0],
            control_1_mm: [15.0, -30.0, -10.0],
            control_2_mm: [30.0, -15.0, -10.0],
            end_mm: [30.0, 0.0, 0.0],
        },
    ];

    let output = kernel.sweep_spatial_profile(&profile, &path).unwrap();
    let repeated = kernel.sweep_spatial_profile(&profile, &path).unwrap();
    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert!(output.body.topology.bounds_mm.min.z < -5.0);
    assert!(output.body.topology.bounds_mm.max.z > 5.0);
}

#[test]
fn xy_spatial_sweep_preserves_legacy_planar_profile_placement() {
    use ketchup_exact::SpatialProfileSegment;

    let backend = ExactBackend::new();
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [-2.0, -1.0],
            end_mm: [5.0, -1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [5.0, -1.0],
            end_mm: [5.0, 3.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [5.0, 3.0],
            end_mm: [-2.0, 3.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-2.0, 3.0],
            end_mm: [-2.0, -1.0],
        },
    ];
    let planar_path = [
        PlanarProfileSegment::Line {
            start_mm: [10.0, 20.0],
            end_mm: [25.0, 20.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [25.0, 20.0],
            end_mm: [40.0, 20.0],
        },
    ];
    let spatial_path = [
        SpatialProfileSegment::Line {
            start_mm: [10.0, 20.0, 0.0],
            end_mm: [25.0, 20.0, 0.0],
        },
        SpatialProfileSegment::Line {
            start_mm: [25.0, 20.0, 0.0],
            end_mm: [40.0, 20.0, 0.0],
        },
    ];

    let planar = backend
        .sweep_planar_profile(&profile, &planar_path)
        .unwrap();
    let spatial = backend
        .sweep_spatial_profile(&profile, &spatial_path)
        .unwrap();
    assert_valid(&planar);
    assert_valid(&spatial);

    let planar_topology = &planar.body.topology;
    let spatial_topology = &spatial.body.topology;
    assert_eq!(
        [
            spatial_topology.vertex_count,
            spatial_topology.edge_count,
            spatial_topology.wire_count,
            spatial_topology.face_count,
            spatial_topology.shell_count,
            spatial_topology.solid_count,
        ],
        [
            planar_topology.vertex_count,
            planar_topology.edge_count,
            planar_topology.wire_count,
            planar_topology.face_count,
            planar_topology.shell_count,
            planar_topology.solid_count,
        ]
    );
    assert_close(spatial_topology.volume_mm3, planar_topology.volume_mm3);
    for (actual, expected) in [
        (
            spatial_topology.bounds_mm.min.x,
            planar_topology.bounds_mm.min.x,
        ),
        (
            spatial_topology.bounds_mm.min.y,
            planar_topology.bounds_mm.min.y,
        ),
        (
            spatial_topology.bounds_mm.min.z,
            planar_topology.bounds_mm.min.z,
        ),
        (
            spatial_topology.bounds_mm.max.x,
            planar_topology.bounds_mm.max.x,
        ),
        (
            spatial_topology.bounds_mm.max.y,
            planar_topology.bounds_mm.max.y,
        ),
        (
            spatial_topology.bounds_mm.max.z,
            planar_topology.bounds_mm.max.z,
        ),
    ] {
        assert_close(actual, expected);
    }
    assert_close(spatial_topology.bounds_mm.min.x, 10.0);
    assert_close(spatial_topology.bounds_mm.min.y, 15.0);
    assert_close(spatial_topology.bounds_mm.min.z, -1.0);
    assert_close(spatial_topology.bounds_mm.max.x, 40.0);
    assert_close(spatial_topology.bounds_mm.max.y, 22.0);
    assert_close(spatial_topology.bounds_mm.max.z, 3.0);

    let ordered_faces = |output: &ExactOpOutput| {
        let mut faces = output.body.topology.faces.clone();
        faces.sort_by(|left, right| {
            left.centroid_mm
                .x
                .total_cmp(&right.centroid_mm.x)
                .then(left.centroid_mm.y.total_cmp(&right.centroid_mm.y))
                .then(left.centroid_mm.z.total_cmp(&right.centroid_mm.z))
                .then(left.area_mm2.total_cmp(&right.area_mm2))
        });
        faces
    };
    for (spatial_face, planar_face) in ordered_faces(&spatial)
        .iter()
        .zip(ordered_faces(&planar).iter())
    {
        assert_eq!(spatial_face.surface_kind, planar_face.surface_kind);
        assert_eq!(spatial_face.edge_count, planar_face.edge_count);
        assert_close(spatial_face.area_mm2, planar_face.area_mm2);
        for (actual, expected) in [
            (spatial_face.centroid_mm.x, planar_face.centroid_mm.x),
            (spatial_face.centroid_mm.y, planar_face.centroid_mm.y),
            (spatial_face.centroid_mm.z, planar_face.centroid_mm.z),
            (spatial_face.bounds_mm.min.x, planar_face.bounds_mm.min.x),
            (spatial_face.bounds_mm.min.y, planar_face.bounds_mm.min.y),
            (spatial_face.bounds_mm.min.z, planar_face.bounds_mm.min.z),
            (spatial_face.bounds_mm.max.x, planar_face.bounds_mm.max.x),
            (spatial_face.bounds_mm.max.y, planar_face.bounds_mm.max.y),
            (spatial_face.bounds_mm.max.z, planar_face.bounds_mm.max.z),
            (spatial_face.normal.x, planar_face.normal.x),
            (spatial_face.normal.y, planar_face.normal.y),
            (spatial_face.normal.z, planar_face.normal.z),
        ] {
            assert_close(actual, expected);
        }
    }
}

#[test]
fn planar_circle_and_segment_loft_produces_one_deterministic_exact_solid() {
    let backend = ExactBackend::new();
    let spec = PlanarLoftSpec {
        sections: vec![
            PlanarLoftSection {
                elevation_mm: 0.0,
                profile: PlanarProfileLoop::Circle {
                    center_mm: [0.0, 0.0],
                    radius_mm: 12.0,
                },
            },
            PlanarLoftSection {
                elevation_mm: 30.0,
                profile: PlanarProfileLoop::Segments(vec![
                    PlanarProfileSegment::Line {
                        start_mm: [-10.0, -6.0],
                        end_mm: [10.0, -6.0],
                    },
                    PlanarProfileSegment::Line {
                        start_mm: [10.0, -6.0],
                        end_mm: [10.0, 6.0],
                    },
                    PlanarProfileSegment::Line {
                        start_mm: [10.0, 6.0],
                        end_mm: [-10.0, 6.0],
                    },
                    PlanarProfileSegment::Line {
                        start_mm: [-10.0, 6.0],
                        end_mm: [-10.0, -6.0],
                    },
                ]),
            },
        ],
    };
    let output = backend.loft_planar_profiles(&spec).unwrap();
    let repeated = backend.loft_planar_profiles(&spec).unwrap();

    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_eq!(output.body.topology.solid_count, 1);
    assert_close(output.body.topology.bounds_mm.min.z, 0.0);
    assert_close(output.body.topology.bounds_mm.max.z, 30.0);
}

#[test]
fn spline_loft_rejects_elevation_beyond_coordinate_limit() {
    let spec = |elevation_mm| SplineLoftSpec {
        sections: vec![
            SplineLoftSection {
                elevation_mm: 0.0,
                control_points_mm: vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
            },
            SplineLoftSection {
                elevation_mm,
                control_points_mm: vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
            },
        ],
    };
    let backend = ExactBackend::new();
    let error = backend
        .loft_spline(&spec(COORDINATE_LIMIT_MM + 0.001))
        .expect_err("Loft elevation outside the exact coordinate envelope must fail closed");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter);
    let error = backend
        .loft_spline(&spec(f64::NAN))
        .expect_err("non-finite Loft elevation must fail closed");
    assert_eq!(error.code, GeometryErrorCode::NonFiniteParameter);
}

#[test]
fn exact_edge_evidence_finds_two_upper_circular_rim_edges_without_ordinals() {
    let backend = ExactBackend::new();
    let output = backend
        .extrude_planar_region(
            &PlanarProfileLoop::Circle {
                center_mm: [12.0, -7.0],
                radius_mm: 10.0,
            },
            &[PlanarProfileLoop::Circle {
                center_mm: [12.0, -7.0],
                radius_mm: 6.0,
            }],
            30.0,
        )
        .unwrap();

    let mut upper_radii = output
        .body
        .topology
        .edges
        .iter()
        .filter(|edge| {
            edge.curve_kind == "circle"
                && edge.closed
                && (edge.centroid_mm.z - 30.0).abs() <= 1.0e-9
                && edge
                    .axis_direction
                    .is_some_and(|axis| axis.z.abs() >= 1.0 - 1.0e-12)
        })
        .map(|edge| {
            assert_close(edge.centroid_mm.x, 12.0);
            assert_close(edge.centroid_mm.y, -7.0);
            assert_eq!(edge.adjacent_face_ordinals.len(), 2);
            let radius = edge.circle_radius_mm.expect("circle radius");
            assert_close(edge.length_mm, 2.0 * std::f64::consts::PI * radius);
            radius
        })
        .collect::<Vec<_>>();
    upper_radii.sort_by(f64::total_cmp);

    assert_eq!(upper_radii, vec![6.0, 10.0]);
}

#[test]
fn all_cubic_oval_extrusion_is_valid_deterministic_and_bounded() {
    let kappa = 4.0 * (2.0_f64.sqrt() - 1.0) / 3.0;
    let x_handle = 20.0 * kappa;
    let y_handle = 10.0 * kappa;
    let profile = [
        PlanarProfileSegment::CubicBezier {
            start_mm: [20.0, 0.0],
            control_1_mm: [20.0, y_handle],
            control_2_mm: [x_handle, 10.0],
            end_mm: [0.0, 10.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, 10.0],
            control_1_mm: [-x_handle, 10.0],
            control_2_mm: [-20.0, y_handle],
            end_mm: [-20.0, 0.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [-20.0, 0.0],
            control_1_mm: [-20.0, -y_handle],
            control_2_mm: [-x_handle, -10.0],
            end_mm: [0.0, -10.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, -10.0],
            control_1_mm: [x_handle, -10.0],
            control_2_mm: [20.0, -y_handle],
            end_mm: [20.0, 0.0],
        },
    ];
    let backend = ExactBackend::new();
    let output = backend.extrude_mixed_profile(&profile, 12.0).unwrap();
    let repeated = backend.extrude_mixed_profile(&profile, 12.0).unwrap();

    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_close(output.body.topology.bounds_mm.min.x, -20.0);
    assert_close(output.body.topology.bounds_mm.min.y, -10.0);
    assert_close(output.body.topology.bounds_mm.min.z, 0.0);
    assert_close(output.body.topology.bounds_mm.max.x, 20.0);
    assert_close(output.body.topology.bounds_mm.max.y, 10.0);
    assert_close(output.body.topology.bounds_mm.max.z, 12.0);
    assert!(
        (7_500.0..7_600.0).contains(&output.body.topology.volume_mm3),
        "unexpected all-cubic oval volume: {}",
        output.body.topology.volume_mm3
    );
}

#[test]
fn compound_region_revolve_preserves_mixed_boundaries_and_hole() {
    let outer = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [10.0, -10.0],
            end_mm: [30.0, -10.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [30.0, -10.0],
            end_mm: [30.0, 10.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [30.0, 10.0],
            control_1_mm: [26.0, 14.0],
            control_2_mm: [22.0, 14.0],
            end_mm: [18.0, 10.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [18.0, 10.0],
            end_mm: [10.0, 2.0],
            center_mm: [18.0, 2.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [10.0, 2.0],
            end_mm: [10.0, -10.0],
        },
    ]);
    let holes = [PlanarProfileLoop::Circle {
        center_mm: [20.0, 0.0],
        radius_mm: 2.0,
    }];
    let backend = ExactBackend::new();
    let first = backend
        .revolve_planar_region(&outer, &holes, [0.0, -20.0], [0.0, 20.0], 270.0)
        .unwrap();
    let repeated = backend
        .revolve_planar_region(&outer, &holes, [0.0, -20.0], [0.0, 20.0], 270.0)
        .unwrap();

    assert_valid(&first);
    assert_eq!(first.input_digest, repeated.input_digest);
    assert_eq!(
        first.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_eq!(first.body.topology.solid_count, 1);
    assert!(first.body.topology.volume_mm3 > 0.0);
    assert!(first.topology_history.iter().any(|entry| {
        entry.semantic_role.as_deref() == Some("revolve.start")
            && entry.output_face_ordinal.is_some()
    }));
    assert!(first.topology_history.iter().any(|entry| {
        entry.semantic_role.as_deref() == Some("revolve.end") && entry.output_face_ordinal.is_some()
    }));

    let reversed_outer = backend
        .revolve_planar_region(
            &reverse_planar_loop(&outer),
            &holes,
            [0.0, -20.0],
            [0.0, 20.0],
            270.0,
        )
        .unwrap();
    assert_valid(&reversed_outer);
    assert_close(
        reversed_outer.body.topology.volume_mm3,
        first.body.topology.volume_mm3,
    );
    assert_eq!(
        reversed_outer.body.topology.face_count,
        first.body.topology.face_count
    );

    let circle_outer = PlanarProfileLoop::Circle {
        center_mm: [20.0, 0.0],
        radius_mm: 10.0,
    };
    let boundary_hole = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [18.0, -2.0],
            end_mm: [22.0, -2.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [22.0, -2.0],
            end_mm: [22.0, 2.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [22.0, 2.0],
            end_mm: [18.0, 2.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [18.0, 2.0],
            end_mm: [18.0, -2.0],
        },
    ]);
    let circle_with_hole = backend
        .revolve_planar_region(
            &circle_outer,
            std::slice::from_ref(&boundary_hole),
            [0.0, -20.0],
            [0.0, 20.0],
            180.0,
        )
        .unwrap();
    let circle_with_reversed_hole = backend
        .revolve_planar_region(
            &circle_outer,
            &[reverse_planar_loop(&boundary_hole)],
            [0.0, -20.0],
            [0.0, 20.0],
            180.0,
        )
        .unwrap();
    assert_valid(&circle_with_hole);
    assert_valid(&circle_with_reversed_hole);
    assert_close(
        circle_with_hole.body.topology.volume_mm3,
        20.0 * std::f64::consts::PI * (100.0 * std::f64::consts::PI - 16.0),
    );
    assert_close(
        circle_with_hole.body.topology.volume_mm3,
        circle_with_reversed_hole.body.topology.volume_mm3,
    );
    assert_eq!(
        circle_with_hole.body.topology.face_count,
        circle_with_reversed_hole.body.topology.face_count
    );

    let invalid_profile = backend
        .revolve_planar_region(
            &PlanarProfileLoop::Segments(vec![PlanarProfileSegment::Line {
                start_mm: [10.0, 0.0],
                end_mm: [20.0, 0.0],
            }]),
            &holes,
            [0.0, -20.0],
            [0.0, 20.0],
            270.0,
        )
        .unwrap_err();
    assert_eq!(invalid_profile.code, GeometryErrorCode::InvalidProfile);
    assert_eq!(invalid_profile.operation, "revolve_planar_region");

    let mut non_finite_outer = outer.clone();
    let PlanarProfileLoop::Segments(segments) = &mut non_finite_outer else {
        unreachable!();
    };
    let PlanarProfileSegment::Line { start_mm, .. } = &mut segments[0] else {
        unreachable!();
    };
    start_mm[0] = f64::NAN;
    let non_finite = backend
        .revolve_planar_region(&non_finite_outer, &holes, [0.0, -20.0], [0.0, 20.0], 270.0)
        .unwrap_err();
    assert_eq!(non_finite.operation, "revolve_planar_region");

    assert_eq!(
        backend
            .revolve_planar_region(&outer, &holes, [0.0, 0.0], [0.0, 0.0], 270.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn step_import_can_address_each_transferred_solid_independently() {
    let backend = ExactBackend::new();
    let first = backend
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
        })
        .unwrap();
    let second = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 40.0,
                y: 50.0,
                z: 60.0,
            },
        })
        .unwrap();
    let assembly = backend.combine_bodies(&first.body, &second.body).unwrap();
    let path = std::env::temp_dir().join(format!(
        "ketchup-step-solid-addressing-{}.step",
        std::process::id()
    ));
    backend
        .export_step(&assembly.body, path.to_str().unwrap())
        .unwrap();

    let solid_0 = backend
        .import_step_solid(path.to_str().unwrap(), 0)
        .unwrap();
    let solid_1 = backend
        .import_step_solid(path.to_str().unwrap(), 1)
        .unwrap();
    let out_of_range = backend
        .import_step_solid(path.to_str().unwrap(), 2)
        .unwrap_err();
    std::fs::remove_file(path).unwrap();

    assert_valid(&solid_0);
    assert_valid(&solid_1);
    assert_close(solid_0.body.topology.volume_mm3, 6_000.0);
    assert_close(solid_1.body.topology.volume_mm3, 120_000.0);
    assert_eq!(out_of_range.code, GeometryErrorCode::InvalidParameter);
}

#[test]
fn iges_round_trip_preserves_exact_body_units_and_rejects_invalid_sources() {
    let backend = ExactBackend::new();
    let source = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -10.0,
                y: 20.0,
                z: 5.0,
            },
            size_mm: Size3 {
                x: 30.0,
                y: 40.0,
                z: 50.0,
            },
        })
        .unwrap();
    let path = std::env::temp_dir().join(format!(
        "ketchup-iges-round-trip-{}.iges",
        std::process::id()
    ));
    backend
        .export_iges(&source.body, path.to_str().unwrap())
        .unwrap();

    let imported = backend.import_iges(path.to_str().unwrap()).unwrap();
    assert_valid(&imported);
    assert_close(imported.body.topology.volume_mm3, 60_000.0);
    assert_eq!(imported.body.topology.solid_count, 1);
    assert_eq!(
        backend.iges_length_unit_name(path.to_str().unwrap()),
        Some("mm".to_owned())
    );

    let malformed =
        path.with_file_name(format!("ketchup-invalid-iges-{}.iges", std::process::id()));
    std::fs::write(&malformed, b"not an IGES model").unwrap();
    let error = backend
        .import_iges(malformed.to_str().unwrap())
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidShape);
    assert_eq!(
        backend.export_iges(&source.body, "").unwrap_err().code,
        GeometryErrorCode::InvalidParameter
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(malformed).unwrap();
}

#[test]
fn cubic_planar_region_extrusion_preserves_hole_bounds_volume_and_fingerprint() {
    let outer = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [-20.0, -15.0],
            end_mm: [20.0, -15.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [20.0, -15.0],
            end_mm: [20.0, 15.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [20.0, 15.0],
            control_1_mm: [10.0, 25.0],
            control_2_mm: [-10.0, 25.0],
            end_mm: [-20.0, 15.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-20.0, 15.0],
            end_mm: [-20.0, -15.0],
        },
    ]);
    let holes = [PlanarProfileLoop::Circle {
        center_mm: [0.0, 0.0],
        radius_mm: 5.0,
    }];
    let backend = ExactBackend::new();
    let output = backend.extrude_planar_region(&outer, &holes, 12.0).unwrap();
    let repeated = backend.extrude_planar_region(&outer, &holes, 12.0).unwrap();

    assert_valid(&output);
    assert_eq!(output.body.topology.solid_count, 1);
    assert_close(
        output.body.topology.volume_mm3,
        (1_410.0 - std::f64::consts::PI * 5.0 * 5.0) * 12.0,
    );
    assert_close(output.body.topology.bounds_mm.min.x, -20.0);
    assert_close(output.body.topology.bounds_mm.min.y, -15.0);
    assert_close(output.body.topology.bounds_mm.min.z, 0.0);
    assert_close(output.body.topology.bounds_mm.max.x, 20.0);
    assert_close(output.body.topology.bounds_mm.max.y, 22.5);
    assert_close(output.body.topology.bounds_mm.max.z, 12.0);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
}

#[test]
fn cubic_sweep_is_deterministic_and_rejects_degenerate_or_backtracking_handles() {
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [-1.0, -1.0],
            end_mm: [1.0, -1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [1.0, -1.0],
            end_mm: [1.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [1.0, 1.0],
            end_mm: [-1.0, 1.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [-1.0, 1.0],
            end_mm: [-1.0, -1.0],
        },
    ];
    let path = [
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [25.0, 0.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [25.0, 0.0],
            control_1_mm: [35.0, 0.0],
            control_2_mm: [45.0, 10.0],
            end_mm: [45.0, 20.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [45.0, 20.0],
            end_mm: [45.0, 45.0],
        },
    ];
    let backend = ExactBackend::new();
    let output = backend.sweep_planar_profile(&profile, &path).unwrap();
    let repeated = backend.sweep_planar_profile(&profile, &path).unwrap();
    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_eq!(output.body.topology.solid_count, 1);
    assert!(output.body.topology.volume_mm3 > 0.0);
    assert!(!output.topology_history.is_empty());

    for invalid_cubic in [
        PlanarProfileSegment::CubicBezier {
            start_mm: [25.0, 0.0],
            control_1_mm: [25.0, 0.0],
            control_2_mm: [45.0, 10.0],
            end_mm: [45.0, 20.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [25.0, 0.0],
            control_1_mm: [20.0, 0.0],
            control_2_mm: [45.0, 10.0],
            end_mm: [45.0, 20.0],
        },
    ] {
        let invalid_path = [path[0], invalid_cubic, path[2]];
        assert_eq!(
            backend
                .sweep_planar_profile(&profile, &invalid_path)
                .unwrap_err()
                .code,
            GeometryErrorCode::InvalidProfile
        );
    }

    let adjacent_self_intersection = [
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, 0.0],
            control_1_mm: [10.0, -5.0],
            control_2_mm: [9.0, 10.0],
            end_mm: [10.0, 10.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [10.0, 10.0],
            control_1_mm: [11.0, 10.0],
            control_2_mm: [-10.0, -20.0],
            end_mm: [20.0, 0.0],
        },
    ];
    assert_eq!(
        backend
            .sweep_planar_profile(&profile, &adjacent_self_intersection)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );

    for invalid_arc_path in [
        vec![
            path[0],
            PlanarProfileSegment::CircularArc {
                start_mm: [25.0, 0.0],
                end_mm: [25.0, 0.0],
                center_mm: [25.0, 10.0],
                clockwise: false,
            },
        ],
        vec![
            path[0],
            PlanarProfileSegment::CircularArc {
                start_mm: [25.0, 0.0],
                end_mm: [35.000_000_002, 10.0],
                center_mm: [25.0, 10.0],
                clockwise: false,
            },
        ],
    ] {
        assert_eq!(
            backend
                .sweep_planar_profile(&profile, &invalid_arc_path)
                .unwrap_err()
                .code,
            GeometryErrorCode::InvalidProfile
        );
    }

    let maximum_cubic_path = (0..64)
        .map(|index| {
            let start = index as f64 * 2.0;
            PlanarProfileSegment::CubicBezier {
                start_mm: [start, 0.0],
                control_1_mm: [start + 0.5, 0.0],
                control_2_mm: [start + 1.5, 0.0],
                end_mm: [start + 2.0, 0.0],
            }
        })
        .collect::<Vec<_>>();
    assert_valid(
        &backend
            .sweep_planar_profile(&profile, &maximum_cubic_path)
            .unwrap(),
    );
}

fn volume_mesh_options(max_tetrahedra: u32) -> ExactVolumeMeshOptions {
    ExactVolumeMeshOptions {
        surface_deflection_mm: 0.25,
        angular_deflection_rad: 0.25,
        max_tetrahedra,
        max_relative_volume_error: 0.03,
        min_tetrahedron_quality: 1.0e-5,
    }
}

#[test]
fn exact_box_volume_mesh_is_manifold_positive_and_analytic() {
    let backend = ExactBackend::new();
    let body = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -10.0,
                y: 3.0,
                z: 7.0,
            },
            size_mm: Size3 {
                x: 20.0,
                y: 10.0,
                z: 5.0,
            },
        })
        .unwrap();
    let mesh = backend
        .volume_mesh_body(&body.body, volume_mesh_options(64))
        .unwrap();
    let repeated = backend
        .volume_mesh_body(&body.body, volume_mesh_options(64))
        .unwrap();

    assert_eq!(mesh.tetrahedra.len(), 12);
    assert_eq!(mesh.boundary_triangles.len(), 12);
    assert_eq!(mesh.vertices_mm.len(), 9);
    assert_close(mesh.exact_volume_mm3, 1_000.0);
    assert_close(mesh.tetrahedral_volume_mm3, 1_000.0);
    assert!(mesh.relative_volume_error <= 1.0e-12);
    assert!(mesh.minimum_signed_volume_mm3 > 0.0);
    assert!(mesh.minimum_quality >= 1.0e-5);
    assert_eq!(mesh.mesh_fingerprint, repeated.mesh_fingerprint);
    assert_eq!(mesh.tetrahedra, repeated.tetrahedra);
    assert_eq!(mesh.boundary_triangles, repeated.boundary_triangles);
}

#[test]
fn exact_cylinder_volume_mesh_reports_bounded_geometric_error() {
    let backend = ExactBackend::new();
    let body = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [4.0, -3.0],
            radius_mm: 10.0,
            height_mm: 20.0,
        })
        .unwrap();
    let coarse = backend
        .volume_mesh_body(&body.body, volume_mesh_options(2_048))
        .unwrap();
    let mut refined_options = volume_mesh_options(4_096);
    refined_options.surface_deflection_mm = 0.08;
    refined_options.angular_deflection_rad = 0.08;
    let refined = backend
        .volume_mesh_body(&body.body, refined_options)
        .unwrap();

    assert!(coarse.tetrahedra.len() > 12);
    assert!(refined.tetrahedra.len() > coarse.tetrahedra.len());
    assert!(refined.relative_volume_error < coarse.relative_volume_error);
    assert!(refined.relative_volume_error <= refined_options.max_relative_volume_error);
    assert_close(refined.exact_volume_mm3, 2_000.0 * std::f64::consts::PI);
    assert!(
        refined
            .boundary_triangles
            .iter()
            .all(|triangle| triangle.face_ordinal < body.body.topology.face_count)
    );
}
