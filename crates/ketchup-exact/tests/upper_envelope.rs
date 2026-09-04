use ketchup_exact::{
    BottleEdgeFinish, BoxSpec, CircleExtrudeSpec, CutMode, CylinderToolSpec, ExactBackend,
    ExactOpOutput, GeometryErrorCode, MAX_PLANAR_REGION_HOLES, PlanarProfileLoop,
    PlanarProfileSegment, Point3, RectangleExtrudeSpec, RectangleOffsetSpec, RectangleSweepSpec,
    ReferenceResolution, Size3, SplineLoftSection, SplineLoftSpec, capture_box_shell_references,
    capture_circle_extrusion_references, capture_circular_through_cut_references,
    capture_general_revolve_references, capture_mixed_profile_extrusion_references,
    capture_planar_offset_reference, capture_rectangular_split_references,
    capture_rectangular_sweep_references, capture_spline_loft_references,
    resolve_subshape_reference,
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
fn translated_high_coordinate_cut_succeeds_without_backend_exception() {
    let backend = ExactBackend::new();
    let base = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 999_800.0,
                y: 999_800.0,
                z: 999_800.0,
            },
            size_mm: Size3 {
                x: 100.0,
                y: 100.0,
                z: 100.0,
            },
        })
        .expect("the translated base box must succeed");

    let output = match backend.cut_box(
        &base.body,
        BoxSpec {
            origin_mm: Point3 {
                x: 999_830.0,
                y: 999_840.0,
                z: 999_790.0,
            },
            size_mm: Size3 {
                x: 20.0,
                y: 30.0,
                z: 120.0,
            },
        },
        CutMode::ThroughAll,
    ) {
        Ok(output) => output,
        Err(error) if error.code == GeometryErrorCode::BackendException => {
            panic!("translated cut raised a backend exception: {error}")
        }
        Err(error) => panic!("translated cut unexpectedly failed: {error}"),
    };

    assert_valid(&output);
    let bounds = output.body.topology.bounds_mm;
    assert_close(bounds.min.x, 999_800.0);
    assert_close(bounds.min.y, 999_800.0);
    assert_close(bounds.min.z, 999_800.0);
    assert_close(bounds.max.x, 999_900.0);
    assert_close(bounds.max.y, 999_900.0);
    assert_close(bounds.max.z, 999_900.0);
    assert_close(output.body.topology.volume_mm3, 940_000.0);
}

#[test]
fn translated_overlapping_box_union_produces_one_larger_exact_solid() {
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

    let output = backend
        .fuse_box(
            &base.body,
            BoxSpec {
                origin_mm: Point3 {
                    x: 80.0,
                    y: 0.0,
                    z: 0.0,
                },
                size_mm: Size3 {
                    x: 40.0,
                    y: 60.0,
                    z: 20.0,
                },
            },
        )
        .unwrap();

    assert_valid(&output);
    assert_eq!(output.body.topology.solid_count, 1);
    assert_close(output.body.topology.bounds_mm.max.x, 120.0);
    assert_close(output.body.topology.volume_mm3, 144_000.0);
}

#[test]
fn rectangular_planar_offset_produces_one_exact_face_with_stable_lineage() {
    let backend = ExactBackend::new();
    for (distance_mm, expected) in [
        (5.0, [[5.0, 15.0], [115.0, 105.0]]),
        (-7.5, [[17.5, 27.5], [102.5, 92.5]]),
    ] {
        let mut output = backend
            .offset_rectangle(RectangleOffsetSpec {
                min_mm: [10.0, 20.0],
                max_mm: [110.0, 100.0],
                distance_mm,
            })
            .unwrap();
        assert!(output.tolerance_report.shape_valid);
        assert!(!output.tolerance_report.accepted_exact_solid);
        assert_eq!(output.body.topology.vertex_count, 4);
        assert_eq!(output.body.topology.edge_count, 4);
        assert_eq!(output.body.topology.face_count, 1);
        assert_eq!(output.body.topology.shell_count, 0);
        assert_eq!(output.body.topology.solid_count, 0);
        assert_close(output.body.topology.volume_mm3, 0.0);
        assert_close(output.body.topology.bounds_mm.min.x, expected[0][0]);
        assert_close(output.body.topology.bounds_mm.min.y, expected[0][1]);
        assert_close(output.body.topology.bounds_mm.max.x, expected[1][0]);
        assert_close(output.body.topology.bounds_mm.max.y, expected[1][1]);
        let face = &output.body.topology.faces[0];
        assert_close(
            face.area_mm2,
            (expected[1][0] - expected[0][0]) * (expected[1][1] - expected[0][1]),
        );
        let reference = capture_planar_offset_reference(&mut output, "701", "703").unwrap();
        assert_eq!(reference.semantic_role, "planar_offset.face");
        assert_eq!(reference.source_element_id, "profile.face");
        assert_eq!(
            reference.stability_class,
            ketchup_exact::StabilityClass::Guaranteed
        );
    }
}

#[test]
fn mixed_line_arc_planar_offset_produces_one_stable_exact_face() {
    let backend = ExactBackend::new();
    let profile = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [0.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: false,
        },
    ]);
    let reversed = reverse_planar_loop(&profile);
    let mut shifted = profile.clone();
    let PlanarProfileLoop::Segments(shifted_segments) = &mut shifted else {
        unreachable!("fixture is segmented");
    };
    shifted_segments.rotate_left(1);
    for distance_mm in [2.0, -2.0] {
        let mut output = backend
            .offset_planar_profile(&profile, distance_mm)
            .unwrap();
        let repeated = backend
            .offset_planar_profile(&reversed, distance_mm)
            .unwrap();
        let shifted_output = backend
            .offset_planar_profile(&shifted, distance_mm)
            .unwrap();

        assert!(output.tolerance_report.shape_valid);
        assert!(!output.tolerance_report.accepted_exact_solid);
        assert_eq!(output.input_digest, repeated.input_digest);
        assert_eq!(output.input_digest, shifted_output.input_digest);
        assert_eq!(
            output.body.result_fingerprint,
            repeated.body.result_fingerprint
        );
        assert_eq!(
            output.body.result_fingerprint,
            shifted_output.body.result_fingerprint
        );
        assert_eq!(
            output.body.topology.bounds_mm,
            repeated.body.topology.bounds_mm
        );
        assert_eq!(
            output.body.topology.bounds_mm,
            shifted_output.body.topology.bounds_mm
        );
        assert_close(
            output.body.topology.faces[0].area_mm2,
            repeated.body.topology.faces[0].area_mm2,
        );
        assert_eq!(output.body.topology.face_count, 1);
        assert_eq!(output.body.topology.shell_count, 0);
        assert_eq!(output.body.topology.solid_count, 0);
        assert!(output.body.topology.edge_count >= 2);
        if distance_mm > 0.0 {
            assert!(output.body.topology.faces[0].area_mm2 > 50.0 * std::f64::consts::PI);
        } else {
            assert!(output.body.topology.faces[0].area_mm2 < 50.0 * std::f64::consts::PI);
        }
        let reference = capture_planar_offset_reference(&mut output, "721", "723").unwrap();
        assert_eq!(reference.semantic_role, "planar_offset.face");
        assert_eq!(reference.source_element_id, "profile.face");
        assert_eq!(
            reference.stability_class,
            ketchup_exact::StabilityClass::Guaranteed
        );
    }

    let mut incomplete_topology = backend.offset_planar_profile(&profile, 2.0).unwrap();
    incomplete_topology.body.topology.edges.clear();
    assert_eq!(
        capture_planar_offset_reference(&mut incomplete_topology, "721", "723")
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidShape
    );
    for invalid_vertex_count in [0, 1] {
        let mut incomplete_vertices = backend.offset_planar_profile(&profile, 2.0).unwrap();
        incomplete_vertices.body.topology.vertex_count = invalid_vertex_count;
        assert_eq!(
            capture_planar_offset_reference(&mut incomplete_vertices, "721", "723")
                .unwrap_err()
                .code,
            GeometryErrorCode::InvalidShape
        );
    }
    let mut non_finite_bounds = backend.offset_planar_profile(&profile, 2.0).unwrap();
    non_finite_bounds.body.topology.bounds_mm.min.x = f64::NAN;
    assert_eq!(
        capture_planar_offset_reference(&mut non_finite_bounds, "721", "723")
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidShape
    );
    let mut non_finite_topology = backend.offset_planar_profile(&profile, 2.0).unwrap();
    non_finite_topology.body.topology.volume_mm3 = f64::NAN;
    assert_eq!(
        capture_planar_offset_reference(&mut non_finite_topology, "721", "723")
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidShape
    );

    let open = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [1.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: false,
        },
    ]);
    assert_eq!(
        backend.offset_planar_profile(&open, 2.0).unwrap_err().code,
        GeometryErrorCode::InvalidProfile
    );
    let zero_area = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::CircularArc {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: true,
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [0.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: false,
        },
    ]);
    assert_eq!(
        backend
            .offset_planar_profile(&zero_area, 2.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    let oversized_arc = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::CircularArc {
            start_mm: [999_950.0, -100.0],
            end_mm: [999_950.0, 100.0],
            center_mm: [999_950.0, 0.0],
            clockwise: false,
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [999_950.0, 100.0],
            end_mm: [999_950.0, -100.0],
            center_mm: [999_950.0, 0.0],
            clockwise: false,
        },
    ]);
    assert_eq!(
        backend
            .offset_planar_profile(&oversized_arc, 2.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    let self_intersecting = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 20.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [20.0, 20.0],
            end_mm: [0.0, 20.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [0.0, 20.0],
            end_mm: [20.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [0.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: true,
        },
    ]);
    assert_eq!(
        backend
            .offset_planar_profile(&self_intersecting, 2.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidShape
    );
    assert_eq!(
        backend
            .offset_planar_profile(&profile, f64::NAN)
            .unwrap_err()
            .code,
        GeometryErrorCode::NonFiniteParameter
    );
}

#[test]
fn cubic_and_mixed_planar_offsets_are_signed_stable_and_fail_closed() {
    let backend = ExactBackend::new();
    let kappa = 4.0 * (2.0_f64.sqrt() - 1.0) / 3.0;
    let cubic = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::CubicBezier {
            start_mm: [20.0, 0.0],
            control_1_mm: [20.0, 10.0 * kappa],
            control_2_mm: [20.0 * kappa, 10.0],
            end_mm: [0.0, 10.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, 10.0],
            control_1_mm: [-20.0 * kappa, 10.0],
            control_2_mm: [-20.0, 10.0 * kappa],
            end_mm: [-20.0, 0.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [-20.0, 0.0],
            control_1_mm: [-20.0, -10.0 * kappa],
            control_2_mm: [-20.0 * kappa, -10.0],
            end_mm: [0.0, -10.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, -10.0],
            control_1_mm: [20.0 * kappa, -10.0],
            control_2_mm: [20.0, -10.0 * kappa],
            end_mm: [20.0, 0.0],
        },
    ]);
    let mixed = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [40.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [40.0, 0.0],
            end_mm: [50.0, 10.0],
            center_mm: [40.0, 10.0],
            clockwise: false,
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [50.0, 10.0],
            control_1_mm: [50.0, 20.0],
            control_2_mm: [0.0, 20.0],
            end_mm: [0.0, 10.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [0.0, 10.0],
            end_mm: [0.0, 0.0],
        },
    ]);

    for profile in [&cubic, &mixed] {
        let mut outward = backend.offset_planar_profile(profile, 2.0).unwrap();
        let inward = backend.offset_planar_profile(profile, -2.0).unwrap();
        assert_eq!(outward.body.topology.face_count, 1);
        assert_eq!(outward.body.topology.shell_count, 0);
        assert_eq!(outward.body.topology.solid_count, 0);
        assert!(outward.body.topology.faces[0].area_mm2 > inward.body.topology.faces[0].area_mm2);
        assert!(outward.tolerance_report.shape_valid);
        assert!(inward.tolerance_report.shape_valid);
        let reference = capture_planar_offset_reference(&mut outward, "721", "725").unwrap();
        assert_eq!(
            reference.stability_class,
            ketchup_exact::StabilityClass::Guaranteed
        );
    }

    assert!(
        backend.offset_planar_profile(&mixed, -10.0).is_err(),
        "collapsed mixed cubic offset must fail closed"
    );

    for profile in [&cubic, &mixed] {
        let reversed = reverse_planar_loop(profile);
        let mut shifted = profile.clone();
        let PlanarProfileLoop::Segments(shifted_segments) = &mut shifted else {
            unreachable!("fixture is segmented");
        };
        shifted_segments.rotate_left(1);
        let canonical = backend.offset_planar_profile(profile, 2.0).unwrap();
        for equivalent in [&reversed, &shifted] {
            let equivalent = backend.offset_planar_profile(equivalent, 2.0).unwrap();
            assert_eq!(canonical.input_digest, equivalent.input_digest);
            assert_eq!(
                canonical.body.result_fingerprint,
                equivalent.body.result_fingerprint
            );
            assert_eq!(
                canonical.body.topology.bounds_mm,
                equivalent.body.topology.bounds_mm
            );
            assert_close(
                canonical.body.topology.faces[0].area_mm2,
                equivalent.body.topology.faces[0].area_mm2,
            );
        }
    }

    let oversized = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, 0.0],
            control_1_mm: [100_000.0, 0.0],
            control_2_mm: [100_000.0, 1.0],
            end_mm: [0.0, 1.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [0.0, 1.0],
            control_1_mm: [1.0, 1.0],
            control_2_mm: [1.0, 0.0],
            end_mm: [0.0, 0.0],
        },
    ]);
    assert_eq!(
        backend
            .offset_planar_profile(&oversized, 2.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );

    let self_intersecting = PlanarProfileLoop::Segments(vec![
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [30.0, 0.0],
        },
        PlanarProfileSegment::CubicBezier {
            start_mm: [30.0, 0.0],
            control_1_mm: [25.0, 20.0],
            control_2_mm: [5.0, -30.0],
            end_mm: [0.0, 20.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [0.0, 20.0],
            end_mm: [0.0, 0.0],
        },
    ]);
    assert_eq!(
        backend
            .offset_planar_profile(&self_intersecting, 2.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidShape
    );
}

#[test]
fn compound_planar_offset_preserves_signed_holes_and_fails_closed() {
    let rectangle = |min: [f64; 2], max: [f64; 2]| {
        PlanarProfileLoop::Segments(vec![
            PlanarProfileSegment::Line {
                start_mm: min,
                end_mm: [max[0], min[1]],
            },
            PlanarProfileSegment::Line {
                start_mm: [max[0], min[1]],
                end_mm: max,
            },
            PlanarProfileSegment::Line {
                start_mm: max,
                end_mm: [min[0], max[1]],
            },
            PlanarProfileSegment::Line {
                start_mm: [min[0], max[1]],
                end_mm: min,
            },
        ])
    };
    let outer = rectangle([-20.0, -15.0], [20.0, 15.0]);
    let holes = vec![
        rectangle([-11.0, -3.0], [-5.0, 3.0]),
        rectangle([5.0, -3.0], [11.0, 3.0]),
    ];
    let backend = ExactBackend::new();

    for (distance_mm, expected_bounds, expected_area) in [
        (2.0, [[-22.0, -17.0], [22.0, 17.0]], 1_488.0),
        (-2.0, [[-18.0, -13.0], [18.0, 13.0]], 736.0),
    ] {
        let mut output = backend
            .offset_planar_region(&outer, &holes, distance_mm)
            .unwrap();
        let repeated = backend
            .offset_planar_region(&outer, &holes, distance_mm)
            .unwrap();
        let reversed_outer = reverse_planar_loop(&outer);
        let reversed_holes = holes
            .iter()
            .rev()
            .map(reverse_planar_loop)
            .collect::<Vec<_>>();
        let equivalent = backend
            .offset_planar_region(&reversed_outer, &reversed_holes, distance_mm)
            .unwrap();

        assert!(output.tolerance_report.shape_valid);
        assert!(!output.tolerance_report.accepted_exact_solid);
        assert_eq!(output.input_digest, repeated.input_digest);
        assert_eq!(output.input_digest, equivalent.input_digest);
        assert_eq!(
            output.body.result_fingerprint,
            repeated.body.result_fingerprint
        );
        assert_eq!(
            output.body.result_fingerprint,
            equivalent.body.result_fingerprint
        );
        assert_eq!(output.body.topology.face_count, 1);
        assert_eq!(output.body.topology.shell_count, 0);
        assert_eq!(output.body.topology.solid_count, 0);
        assert_eq!(output.body.topology.edge_count, 12);
        assert_close(output.body.topology.bounds_mm.min.x, expected_bounds[0][0]);
        assert_close(output.body.topology.bounds_mm.min.y, expected_bounds[0][1]);
        assert_close(output.body.topology.bounds_mm.max.x, expected_bounds[1][0]);
        assert_close(output.body.topology.bounds_mm.max.y, expected_bounds[1][1]);
        assert_close(output.body.topology.faces[0].area_mm2, expected_area);
        let reference = capture_planar_offset_reference(&mut output, "731", "733").unwrap();
        assert_eq!(reference.semantic_role, "planar_offset.face");
        assert_eq!(reference.source_element_id, "profile.face");
        assert_eq!(
            reference.stability_class,
            ketchup_exact::StabilityClass::Guaranteed
        );
    }

    let positive_zero = backend
        .offset_planar_region(
            &outer,
            &[PlanarProfileLoop::Circle {
                center_mm: [0.0, 0.0],
                radius_mm: 4.0,
            }],
            1.0,
        )
        .unwrap();
    let negative_zero = backend
        .offset_planar_region(
            &outer,
            &[PlanarProfileLoop::Circle {
                center_mm: [-0.0, -0.0],
                radius_mm: 4.0,
            }],
            1.0,
        )
        .unwrap();
    assert_eq!(positive_zero.input_digest, negative_zero.input_digest);
    assert_eq!(
        positive_zero.body.result_fingerprint,
        negative_zero.body.result_fingerprint
    );

    assert_eq!(
        backend
            .offset_planar_region(&outer, &[], 2.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    assert_eq!(
        backend
            .offset_planar_region(&outer, &holes, f64::NAN)
            .unwrap_err()
            .code,
        GeometryErrorCode::NonFiniteParameter
    );
    assert!(
        backend
            .offset_planar_region(
                &outer,
                &[PlanarProfileLoop::Circle {
                    center_mm: [0.0, 0.0],
                    radius_mm: 2.0,
                }],
                2.0,
            )
            .is_err(),
        "collapsed hole must fail closed"
    );
    assert!(
        backend.offset_planar_region(&outer, &holes, -7.0).is_err(),
        "expanded holes crossing the contracted outer loop must fail closed"
    );
    assert!(
        backend
            .offset_planar_region(
                &outer,
                &[PlanarProfileLoop::Circle {
                    center_mm: [30.0, 0.0],
                    radius_mm: 2.0,
                }],
                1.0,
            )
            .is_err(),
        "a hole outside the outer loop must fail closed"
    );
    assert!(
        backend
            .offset_planar_region(
                &outer,
                &[
                    PlanarProfileLoop::Circle {
                        center_mm: [-1.0, 0.0],
                        radius_mm: 3.0,
                    },
                    PlanarProfileLoop::Circle {
                        center_mm: [1.0, 0.0],
                        radius_mm: 3.0,
                    },
                ],
                1.0,
            )
            .is_err(),
        "overlapping source holes must fail closed"
    );
    let too_many_holes = (0..=MAX_PLANAR_REGION_HOLES)
        .map(|index| PlanarProfileLoop::Circle {
            center_mm: [index as f64, 0.0],
            radius_mm: 0.1,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        backend
            .offset_planar_region(&outer, &too_many_holes, 1.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
}

#[test]
fn circular_planar_offset_preserves_signed_radius_and_stable_exact_face() {
    let backend = ExactBackend::new();
    let profile = PlanarProfileLoop::Circle {
        center_mm: [12.0, -8.0],
        radius_mm: 20.0,
    };

    for (distance_mm, expected_radius_mm) in [(3.0, 23.0), (-3.0, 17.0)] {
        let mut output = backend
            .offset_planar_profile(&profile, distance_mm)
            .unwrap();
        let repeated = backend
            .offset_planar_profile(&profile, distance_mm)
            .unwrap();
        let bounds = output.body.topology.bounds_mm;

        assert_eq!(output.input_digest, repeated.input_digest);
        assert_eq!(
            output.body.result_fingerprint,
            repeated.body.result_fingerprint
        );
        assert_eq!(output.body.topology.face_count, 1);
        assert_eq!(output.body.topology.shell_count, 0);
        assert_eq!(output.body.topology.solid_count, 0);
        assert_eq!(output.body.topology.edge_count, 1);
        assert_eq!(output.body.topology.vertex_count, 1);
        assert_close(bounds.min.x, 12.0 - expected_radius_mm);
        assert_close(bounds.min.y, -8.0 - expected_radius_mm);
        assert_close(bounds.max.x, 12.0 + expected_radius_mm);
        assert_close(bounds.max.y, -8.0 + expected_radius_mm);
        assert_close(
            output.body.topology.faces[0].area_mm2,
            std::f64::consts::PI * expected_radius_mm * expected_radius_mm,
        );
        let reference = capture_planar_offset_reference(&mut output, "721", "724").unwrap();
        assert_eq!(reference.semantic_role, "planar_offset.face");
        assert_eq!(reference.source_element_id, "profile.face");
        assert_eq!(
            reference.stability_class,
            ketchup_exact::StabilityClass::Guaranteed
        );
    }

    for (profile, distance_mm) in [
        (
            PlanarProfileLoop::Circle {
                center_mm: [0.0, 0.0],
                radius_mm: 2.0,
            },
            -2.0,
        ),
        (
            PlanarProfileLoop::Circle {
                center_mm: [0.0, 0.0],
                radius_mm: 2.0,
            },
            -1.991,
        ),
        (
            PlanarProfileLoop::Circle {
                center_mm: [999_950.0, 0.0],
                radius_mm: 50.0,
            },
            1.0,
        ),
    ] {
        assert_eq!(
            backend
                .offset_planar_profile(&profile, distance_mm)
                .unwrap_err()
                .code,
            GeometryErrorCode::InvalidParameter
        );
    }
    assert_eq!(
        backend
            .offset_planar_profile(
                &PlanarProfileLoop::Circle {
                    center_mm: [f64::NAN, 0.0],
                    radius_mm: 2.0,
                },
                1.0,
            )
            .unwrap_err()
            .code,
        GeometryErrorCode::NonFiniteParameter
    );
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
        8.0 * (50.0 + 25.0 * std::f64::consts::FRAC_PI_2),
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
}

#[test]
fn rectangular_sweep_follows_selected_path_with_stable_exact_faces() {
    let backend = ExactBackend::new();
    let spec = RectangleSweepSpec {
        profile_min_mm: [-5.0, -10.0],
        profile_max_mm: [5.0, 10.0],
        path_start_mm: [0.0, 0.0],
        path_end_mm: [75.0, 100.0],
    };
    let output = backend.sweep_rectangle(spec).unwrap();
    let repeated = backend.sweep_rectangle(spec).unwrap();

    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_eq!(
        [
            output.body.topology.vertex_count,
            output.body.topology.edge_count,
            output.body.topology.face_count,
            output.body.topology.shell_count,
            output.body.topology.solid_count,
        ],
        [8, 12, 6, 1, 1]
    );
    assert_close(output.body.topology.volume_mm3, 25_000.0);
    let references = capture_rectangular_sweep_references(&output, "711", "714").unwrap();
    assert_eq!(references.len(), 6);
    assert_eq!(
        references
            .iter()
            .map(|reference| reference.semantic_role.as_str())
            .collect::<Vec<_>>(),
        [
            "sweep.start",
            "sweep.end",
            "sweep.side.0",
            "sweep.side.1",
            "sweep.side.2",
            "sweep.side.3",
        ]
    );
    assert!(references.iter().all(|reference| {
        reference.stability_class == ketchup_exact::StabilityClass::Guaranteed
            && !reference.lineage_digest.is_empty()
            && !reference.corroborating_geometry_fingerprint.is_empty()
    }));
}

#[test]
fn rectangular_sweep_rejects_transformed_output_beyond_coordinate_limit() {
    let error = ExactBackend::new()
        .sweep_rectangle(RectangleSweepSpec {
            profile_min_mm: [0.0, 0.0],
            profile_max_mm: [10.0, 10.0],
            path_start_mm: [COORDINATE_LIMIT_MM, 0.0],
            path_end_mm: [COORDINATE_LIMIT_MM, 100.0],
        })
        .expect_err("transformed sweep corners outside the exact envelope must fail closed");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter);
}

#[test]
fn bounded_spline_loft_is_deterministic_with_stable_exact_faces() {
    let backend = ExactBackend::new();
    let spec = SplineLoftSpec {
        sections: vec![
            SplineLoftSection {
                elevation_mm: 0.0,
                control_points_mm: vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
            },
            SplineLoftSection {
                elevation_mm: 80.0,
                control_points_mm: vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
            },
        ],
    };
    let output = backend.loft_spline(&spec).unwrap();
    let repeated = backend.loft_spline(&spec).unwrap();

    assert_valid(&output);
    assert_eq!(output.input_digest, repeated.input_digest);
    assert_eq!(
        output.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_eq!(output.body.topology.face_count, 3);
    assert_eq!(output.body.topology.shell_count, 1);
    assert_eq!(output.body.topology.solid_count, 1);
    assert_close(output.body.topology.bounds_mm.min.z, 0.0);
    assert_close(output.body.topology.bounds_mm.max.z, 80.0);
    let references = capture_spline_loft_references(&output, "721", "724").unwrap();
    assert_eq!(
        references
            .iter()
            .map(|reference| reference.semantic_role.as_str())
            .collect::<Vec<_>>(),
        ["loft.start", "loft.end", "loft.side"]
    );
    assert!(references.iter().all(|reference| {
        !reference.lineage_digest.is_empty()
            && !reference.corroborating_geometry_fingerprint.is_empty()
    }));
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
fn overlapping_box_split_preserves_target_as_closed_exact_fragments() {
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

    let mut output = backend
        .split_box(
            &base.body,
            BoxSpec {
                origin_mm: Point3 {
                    x: 70.0,
                    y: 20.0,
                    z: 0.0,
                },
                size_mm: Size3 {
                    x: 60.0,
                    y: 40.0,
                    z: 20.0,
                },
            },
        )
        .unwrap();

    assert!(output.tolerance_report.shape_valid);
    assert!(output.tolerance_report.accepted_exact_solid);
    assert_eq!(output.body.topology.solid_count, 2);
    assert_eq!(output.body.topology.shell_count, 2);
    assert_close(output.body.topology.volume_mm3, 120_000.0);
    assert_close(output.body.topology.bounds_mm.min.x, 0.0);
    assert_close(output.body.topology.bounds_mm.max.x, 100.0);
    let references = capture_rectangular_split_references(&mut output, "1", "4").unwrap();
    assert_eq!(references.len(), 3);
    assert!(references.iter().all(|reference| {
        !reference.lineage_digest.is_empty()
            && !reference.corroborating_geometry_fingerprint.is_empty()
    }));
}

#[test]
fn disjoint_box_union_fails_closed() {
    let backend = ExactBackend::new();
    let base = backend
        .make_box(BoxSpec {
            origin_mm: Point3::ORIGIN,
            size_mm: Size3 {
                x: 10.0,
                y: 10.0,
                z: 10.0,
            },
        })
        .unwrap();

    let error = backend
        .fuse_box(
            &base.body,
            BoxSpec {
                origin_mm: Point3 {
                    x: 20.0,
                    y: 0.0,
                    z: 0.0,
                },
                size_mm: Size3 {
                    x: 5.0,
                    y: 5.0,
                    z: 5.0,
                },
            },
        )
        .unwrap_err();

    assert_eq!(error.code, GeometryErrorCode::NoGeometricChange);
    assert_eq!(error.operation, "fuse_box");
}

#[test]
fn exact_circle_extrusion_has_analytic_volume_and_stable_cylindrical_side() {
    let output = ExactBackend::new()
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [25.0, -15.0],
            radius_mm: 10.0,
            height_mm: 20.0,
        })
        .unwrap();

    assert_valid(&output);
    assert_close(
        output.body.topology.volume_mm3,
        std::f64::consts::PI * 2_000.0,
    );
    assert_close(output.body.topology.bounds_mm.min.x, 15.0);
    assert_close(output.body.topology.bounds_mm.max.y, -5.0);
    let references =
        capture_circle_extrusion_references(&output, "circle-doc", "circle-extrude").unwrap();
    assert_eq!(references.len(), 3);
    assert_eq!(
        references
            .iter()
            .find(|reference| reference.semantic_role == "extrusion.side(profile_edge=circle)")
            .unwrap()
            .expected_type,
        "cylindrical_face"
    );
}

#[test]
fn exact_mixed_line_arc_profile_uses_analytic_arc_and_rejects_open_wire() {
    let backend = ExactBackend::new();
    let segments = [
        PlanarProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [20.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [0.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: false,
        },
    ];
    let output = backend.extrude_mixed_profile(&segments, 8.0).unwrap();
    assert_close(
        output.body.topology.volume_mm3,
        400.0 * std::f64::consts::PI,
    );
    assert_close(output.body.topology.bounds_mm.min.x, 0.0);
    assert_close(output.body.topology.bounds_mm.min.y, 0.0);
    assert_close(output.body.topology.bounds_mm.max.x, 20.0);
    assert_close(output.body.topology.bounds_mm.max.y, 10.0);
    let references =
        capture_mixed_profile_extrusion_references(&output, "mixed-doc", "mixed-extrude").unwrap();
    assert_eq!(references.len(), 3);
    assert_eq!(
        references
            .iter()
            .find(|reference| { reference.semantic_role == "extrusion.side(profile_edge=arc.0)" })
            .unwrap()
            .expected_type,
        "face"
    );

    let open = [
        segments[0],
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [1.0, 0.0],
            center_mm: [10.0, 0.0],
            clockwise: false,
        },
    ];
    assert_eq!(
        backend.extrude_mixed_profile(&open, 8.0).unwrap_err().code,
        GeometryErrorCode::InvalidProfile
    );
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
fn exact_line_arc_d_profile_cuts_and_intersects_a_rectangular_body_and_rejects_broader_curves() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 18.0,
        })
        .unwrap();
    let d_profile = [
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 20.0],
            end_mm: [40.0, 20.0],
            center_mm: [30.0, 20.0],
            clockwise: true,
        },
        PlanarProfileSegment::Line {
            start_mm: [40.0, 20.0],
            end_mm: [20.0, 20.0],
        },
    ];
    let cut = backend
        .cut_mixed_profile(&base.body, &d_profile, -1.0, 20.0)
        .unwrap();
    assert!(
        (cut.body.topology.volume_mm3 - (108_000.0 - 900.0 * std::f64::consts::PI)).abs() < 1.0e-3
    );
    assert_eq!(cut.body.topology.solid_count, 1);
    assert!(cut.topology_history.iter().any(|entry| {
        entry.semantic_role.as_deref() == Some("through_cut.wall.line.0")
            && entry.source_element_id == "cut_profile.edge.line.0"
            && entry.output_face_ordinal.is_some()
    }));

    let pocket = backend
        .cut_mixed_profile(&base.body, &d_profile, 10.0, 8.0)
        .unwrap();
    assert!(
        (pocket.body.topology.volume_mm3 - (108_000.0 - 400.0 * std::f64::consts::PI)).abs()
            < 1.0e-3
    );
    assert_eq!(pocket.body.topology.solid_count, 1);
    assert_eq!(pocket.body.topology.face_count, 9);

    let intersection = backend
        .common_mixed_profile(&base.body, &d_profile, 0.0, 18.0)
        .unwrap();
    assert!((intersection.body.topology.volume_mm3 - 900.0 * std::f64::consts::PI).abs() < 1.0e-3);
    assert_eq!(intersection.body.topology.solid_count, 1);
    assert_eq!(intersection.body.topology.face_count, 4);

    let split = backend
        .split_mixed_profile(&base.body, &d_profile, 0.0, 18.0)
        .unwrap();
    assert_close(split.body.topology.volume_mm3, 108_000.0);
    assert_eq!(split.body.topology.solid_count, 2);
    assert_eq!(split.body.topology.shell_count, 2);
    assert!(
        split
            .body
            .topology
            .faces
            .iter()
            .any(|face| face.surface_kind == "other")
    );

    let containing_d_profile = [
        PlanarProfileSegment::CircularArc {
            start_mm: [-20.0, -100.0],
            end_mm: [-20.0, 160.0],
            center_mm: [-20.0, 30.0],
            clockwise: false,
        },
        PlanarProfileSegment::Line {
            start_mm: [-20.0, 160.0],
            end_mm: [-20.0, -100.0],
        },
    ];
    let union = backend
        .fuse_mixed_profile(&base.body, &containing_d_profile, 0.0, 18.0)
        .unwrap();
    assert!(
        (union.body.topology.volume_mm3 - 0.5 * std::f64::consts::PI * 130.0 * 130.0 * 18.0).abs()
            < 1.0e-3
    );
    assert_eq!(union.body.topology.solid_count, 1);
    assert_eq!(union.body.topology.face_count, 4);

    let broader = [
        d_profile[0],
        PlanarProfileSegment::CircularArc {
            start_mm: [40.0, 20.0],
            end_mm: [20.0, 20.0],
            center_mm: [30.0, 20.0],
            clockwise: true,
        },
    ];
    assert_eq!(
        backend
            .cut_mixed_profile(&base.body, &broader, -1.0, 20.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    assert_eq!(
        backend
            .common_mixed_profile(&base.body, &broader, 0.0, 18.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    assert_eq!(
        backend
            .fuse_mixed_profile(&base.body, &broader, 0.0, 18.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
    assert_eq!(
        backend
            .split_mixed_profile(&base.body, &broader, 0.0, 18.0)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidProfile
    );
}

#[test]
fn box_shell_fillet_and_chamfer_are_exact_deterministic_and_keep_stable_faces() {
    let backend = ExactBackend::new();
    let spec = RectangleExtrudeSpec {
        width_mm: 80.0,
        depth_mm: 50.0,
        height_mm: 30.0,
    };
    let shell = backend.shell_box(spec, 2.0).unwrap();
    let repeated = backend.shell_box(spec, 2.0).unwrap();
    assert_valid(&shell);
    assert_eq!(shell.input_digest, repeated.input_digest);
    assert_eq!(
        shell.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_close(
        shell.body.topology.volume_mm3,
        120_000.0 - 76.0 * 46.0 * 28.0,
    );
    let shell_refs = capture_box_shell_references(&shell, "box-doc", "box-shell").unwrap();
    assert_eq!(shell_refs.len(), 3);
    assert!(shell_refs.iter().all(|reference| {
        reference.stability_class == ketchup_exact::StabilityClass::Guaranteed
            && !reference.lineage_digest.is_empty()
            && !reference.corroborating_geometry_fingerprint.is_empty()
    }));

    for finish in [BottleEdgeFinish::Fillet, BottleEdgeFinish::Chamfer] {
        let output = backend.finish_shell_box(spec, 2.0, finish, 1.0).unwrap();
        let repeated = backend.finish_shell_box(spec, 2.0, finish, 1.0).unwrap();
        assert_valid(&output);
        assert_eq!(output.input_digest, repeated.input_digest);
        assert_eq!(
            output.body.result_fingerprint,
            repeated.body.result_fingerprint
        );
        let references = capture_box_shell_references(&output, "box-doc", "box-finish").unwrap();
        assert_eq!(references.len(), 3);
    }
}

#[test]
fn topology_selected_shell_fillet_and_chamfer_apply_to_an_existing_exact_body() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 37.0,
            depth_mm: 23.0,
            height_mm: 19.0,
        })
        .unwrap();
    let removed_face = base
        .body
        .topology
        .faces
        .iter()
        .find(|face| (face.centroid_mm.z - 19.0).abs() <= 1.0e-6)
        .unwrap()
        .ordinal;

    let shell = backend
        .shell_body(&base.body, &[removed_face], 1.5)
        .unwrap();
    let repeated_shell = backend
        .shell_body(&base.body, &[removed_face], 1.5)
        .unwrap();
    assert_valid(&shell);
    assert!(shell.body.topology.volume_mm3 < base.body.topology.volume_mm3);
    assert_eq!(shell.input_digest, repeated_shell.input_digest);
    assert_eq!(
        shell.body.result_fingerprint,
        repeated_shell.body.result_fingerprint
    );
    assert!(shell.topology_history.iter().any(|entry| {
        entry.source_element_id == format!("generated-result/face/{removed_face}")
            && entry.relation.starts_with("shell_selected_")
    }));

    let selected_edge = base
        .body
        .topology
        .edges
        .iter()
        .find(|edge| edge.adjacent_face_ordinals.len() == 2)
        .unwrap()
        .ordinal;
    for finish in [BottleEdgeFinish::Fillet, BottleEdgeFinish::Chamfer] {
        let output = backend
            .finish_body(&base.body, &[selected_edge], finish, 0.75)
            .unwrap();
        let repeated = backend
            .finish_body(&base.body, &[selected_edge], finish, 0.75)
            .unwrap();
        assert_valid(&output);
        assert_eq!(output.input_digest, repeated.input_digest);
        assert_eq!(
            output.body.result_fingerprint,
            repeated.body.result_fingerprint
        );
        assert!(output.topology_history.iter().any(|entry| {
            entry.source_element_id == format!("generated-result/edge/{selected_edge}")
                && (entry.relation.starts_with("fillet_selected_")
                    || entry.relation.starts_with("chamfer_selected_"))
        }));
    }

    assert_eq!(
        backend
            .shell_body(&base.body, &[removed_face, removed_face], 1.5)
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
    assert_eq!(
        backend
            .finish_body(
                &base.body,
                &[base.body.topology.edge_count],
                BottleEdgeFinish::Fillet,
                0.75,
            )
            .unwrap_err()
            .code,
        GeometryErrorCode::InvalidParameter
    );
}

#[test]
fn general_polygon_revolve_honours_axis_angle_and_stable_roles() {
    let backend = ExactBackend::new();
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [10.0, 0.0],
            end_mm: [20.0, 0.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [20.0, 0.0],
            end_mm: [20.0, 5.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [20.0, 5.0],
            end_mm: [10.0, 5.0],
        },
        PlanarProfileSegment::Line {
            start_mm: [10.0, 5.0],
            end_mm: [10.0, 0.0],
        },
    ];
    let first = backend
        .revolve_general_profile(&profile, [0.0, -10.0], [0.0, 10.0], 180.0)
        .unwrap();
    let repeated = backend
        .revolve_general_profile(&profile, [0.0, -10.0], [0.0, 10.0], 180.0)
        .unwrap();

    assert_valid(&first);
    assert_eq!(first.input_digest, repeated.input_digest);
    assert_eq!(
        first.body.result_fingerprint,
        repeated.body.result_fingerprint
    );
    assert_close(first.body.topology.volume_mm3, 750.0 * std::f64::consts::PI);
    let references =
        capture_general_revolve_references(&first, "general-doc", "polygon-revolve", true).unwrap();
    assert_eq!(references.len(), 4);
    assert_eq!(
        references
            .iter()
            .map(|reference| reference.semantic_role.as_str())
            .collect::<Vec<_>>(),
        [
            "revolve.side.0",
            "revolve.side.1",
            "revolve.start",
            "revolve.end",
        ]
    );
    assert!(references.iter().all(|reference| {
        reference.stability_class == ketchup_exact::StabilityClass::Guaranteed
            && !reference.lineage_digest.is_empty()
            && !reference.corroborating_geometry_fingerprint.is_empty()
    }));
}

#[test]
fn general_segment_profile_revolve_preserves_analytic_arc_evidence() {
    let profile = [
        PlanarProfileSegment::Line {
            start_mm: [10.0, 0.0],
            end_mm: [20.0, 0.0],
        },
        PlanarProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [10.0, 0.0],
            center_mm: [15.0, 0.0],
            clockwise: false,
        },
    ];
    let output = ExactBackend::new()
        .revolve_general_profile(&profile, [0.0, -10.0], [0.0, 10.0], 120.0)
        .unwrap();

    assert_valid(&output);
    assert!(
        output
            .body
            .topology
            .faces
            .iter()
            .any(|face| face.surface_kind != "plane")
    );
    let references =
        capture_general_revolve_references(&output, "general-doc", "arc-revolve", true).unwrap();
    assert_eq!(references.len(), 4);
    assert_eq!(references[1].source_element_id, "profile.edge.1");
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
fn exact_circular_through_cut_has_stable_wall_and_rejects_invalid_radius() {
    let backend = ExactBackend::new();
    let base_spec = RectangleExtrudeSpec {
        width_mm: 100.0,
        depth_mm: 60.0,
        height_mm: 20.0,
    };
    let base = backend.extrude_rectangle(base_spec).unwrap();
    let mut cut = backend
        .cut_cylinder(
            &base.body,
            CylinderToolSpec {
                center_mm: [40.0, 30.0],
                origin_z_mm: -1.0,
                radius_mm: 8.0,
                height_mm: 22.0,
            },
            CutMode::ThroughAll,
        )
        .unwrap();

    assert_valid(&cut);
    assert_close(
        cut.body.topology.volume_mm3,
        120_000.0 - std::f64::consts::PI * 64.0 * 20.0,
    );
    let references = capture_circular_through_cut_references(
        &mut cut,
        "circle-cut-doc",
        "circle-cut",
        base_spec,
    )
    .unwrap();
    assert_eq!(references.len(), 4);
    assert_eq!(
        references
            .iter()
            .find(|reference| reference.semantic_role == "through_cut.wall.circle")
            .unwrap()
            .expected_type,
        "cylindrical_face"
    );

    let error = backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [0.0, 0.0],
            radius_mm: 0.0,
            height_mm: 20.0,
        })
        .unwrap_err();
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter);
}

#[test]
fn exact_containing_cylinder_union_reduces_to_one_valid_cylinder() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 20.0,
        })
        .unwrap();
    let output = backend
        .fuse_cylinder(
            &base.body,
            CylinderToolSpec {
                center_mm: [50.0, 30.0],
                origin_z_mm: 0.0,
                radius_mm: 70.0,
                height_mm: 20.0,
            },
        )
        .unwrap();

    assert_valid(&output);
    assert_close(
        output.body.topology.volume_mm3,
        std::f64::consts::PI * 70.0 * 70.0 * 20.0,
    );
    assert_eq!(output.body.topology.solid_count, 1);
    assert_eq!(output.body.topology.shell_count, 1);
    assert_eq!(output.body.topology.face_count, 3);
}

#[test]
fn exact_contained_cylinder_split_preserves_volume_and_creates_two_solids() {
    let backend = ExactBackend::new();
    let base = backend
        .extrude_rectangle(RectangleExtrudeSpec {
            width_mm: 100.0,
            depth_mm: 60.0,
            height_mm: 20.0,
        })
        .unwrap();
    let mut split = backend
        .split_cylinder(
            &base.body,
            CylinderToolSpec {
                center_mm: [40.0, 30.0],
                origin_z_mm: 0.0,
                radius_mm: 8.0,
                height_mm: 20.0,
            },
        )
        .unwrap();

    assert!(split.tolerance_report.shape_valid);
    assert_close(split.body.topology.volume_mm3, 120_000.0);
    assert_eq!(split.body.topology.solid_count, 2);
    assert_eq!(split.body.topology.shell_count, 2);
    assert_eq!(
        (
            split.body.topology.vertex_count,
            split.body.topology.edge_count,
            split.body.topology.face_count,
            split.body.topology.shell_count,
            split.body.topology.solid_count
        ),
        (10, 15, 9, 2, 2)
    );
    let references =
        capture_rectangular_split_references(&mut split, "circle-split-doc", "circle-split")
            .unwrap();
    assert_eq!(references.len(), 3);
    for reference in references {
        let resolution = resolve_subshape_reference(&reference, &split);
        assert!(
            matches!(
                resolution,
                ReferenceResolution::Resolved {
                    migrated_backend: false,
                    ..
                }
            ),
            "{} resolved as {resolution:?}",
            reference.semantic_role
        );
    }
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
