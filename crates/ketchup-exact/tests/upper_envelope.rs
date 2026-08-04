use ketchup_exact::{
    BoxSpec, CutMode, ExactBackend, ExactOpOutput, GeometryErrorCode, Point3, Size3,
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
