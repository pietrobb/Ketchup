//! Moving one control point of a surface (or changing its weight) reshapes the
//! surface; every seam the caller asks to keep is measured before and after,
//! and a move that would break one is refused with what broke.

use ketchup_exact::{
    ExactBackend, ExactBody, GeometryErrorCode, LoftSurfaceContinuity, SpatialProfileSegment,
    SurfaceControlPoint,
};

const WIDTH: f64 = 50.0;

fn plate(backend: &ExactBackend, x: [f64; 2], z: f64) -> ExactBody {
    let corners = [
        [x[0], 0.0, z],
        [x[1], 0.0, z],
        [x[1], WIDTH, z],
        [x[0], WIDTH, z],
    ];
    let segments: Vec<_> = (0..4)
        .map(|index| SpatialProfileSegment::Line {
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect();
    backend
        .fill_boundary_surface(&segments)
        .expect("plate")
        .body
}

/// A low plate, a high plate and a tangent S blend between them. The blend
/// has six rows of control points across the gap: rows 0 and 1 hold its
/// tangent plane at the low plate, rows 4 and 5 at the high one.
fn s_blend(backend: &ExactBackend) -> (ExactBody, ExactBody, ExactBody) {
    let low = plate(backend, [0.0, 50.0], 0.0);
    let high = plate(backend, [80.0, 130.0], 20.0);
    let blend = backend
        .blend_surfaces(
            &low,
            LoftSurfaceContinuity::Tangent,
            &high,
            LoftSurfaceContinuity::Tangent,
        )
        .expect("blend")
        .body;
    (low, high, blend)
}

fn lift(u_index: u32, v_index: u32, z: f64) -> SurfaceControlPoint {
    SurfaceControlPoint {
        u_index,
        v_index,
        offset_mm: [0.0, 0.0, z],
        weight: None,
    }
}

#[test]
fn moving_an_inner_control_point_keeps_the_tangent_seams() {
    let backend = ExactBackend::new();
    let (low, high, blend) = s_blend(&backend);
    let tangent = LoftSurfaceContinuity::Tangent;
    let moved = backend
        .move_surface_control_point(
            &blend,
            lift(2, 10, 5.0),
            &[(&low, tangent), (&high, tangent)],
        )
        .expect("inner move")
        .body;
    assert_eq!(
        (moved.topology.solid_count, moved.topology.face_count),
        (0, 1)
    );
    for (name, neighbour) in [("low", &low), ("high", &high)] {
        let seam = backend.measure_seam(&moved, neighbour).expect(name);
        assert!((seam.length_mm - WIDTH).abs() < 1.0e-3, "{name}: {seam:?}");
        assert!(seam.gap_mm < 1.0e-3, "{name}: {seam:?}");
        assert!(seam.normal_angle_rad < 1.0e-3, "{name}: {seam:?}");
    }
    // Row 2 shapes the curvature at the low plate, which the move changed.
    let before = backend.measure_seam(&blend, &low).expect("before");
    let after = backend.measure_seam(&moved, &low).expect("after");
    assert!(
        after.curvature_difference_per_mm > before.curvature_difference_per_mm + 1.0e-3,
        "{before:?} -> {after:?}"
    );
}

#[test]
fn a_move_that_would_tilt_a_kept_seam_is_refused_with_the_reason() {
    let backend = ExactBackend::new();
    let (low, _, blend) = s_blend(&backend);
    let error = backend
        .move_surface_control_point(
            &blend,
            lift(1, 10, 5.0),
            &[(&low, LoftSurfaceContinuity::Tangent)],
        )
        .expect_err("row 1 tilts the low seam");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
    assert!(error.diagnostic.contains("normals"), "{error:?}");

    // Without a seam to keep, the same move is made and the seam tilts.
    let moved = backend
        .move_surface_control_point(&blend, lift(1, 10, 5.0), &[])
        .expect("free move")
        .body;
    let seam = backend.measure_seam(&moved, &low).expect("seam");
    assert!(seam.gap_mm < 1.0e-3, "{seam:?}");
    assert!(seam.normal_angle_rad > 1.0e-2, "{seam:?}");
}

#[test]
fn a_seam_the_surfaces_do_not_hold_now_cannot_be_kept() {
    let backend = ExactBackend::new();
    let low = plate(&backend, [0.0, 50.0], 0.0);
    let high = plate(&backend, [80.0, 130.0], 20.0);
    let blend = backend
        .blend_surfaces(
            &low,
            LoftSurfaceContinuity::Position,
            &high,
            LoftSurfaceContinuity::Position,
        )
        .expect("position blend")
        .body;
    let error = backend
        .move_surface_control_point(
            &blend,
            lift(2, 10, 1.0),
            &[(&low, LoftSurfaceContinuity::Curvature)],
        )
        .expect_err("not curvature continuous before the move");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
    assert!(error.diagnostic.contains("before"), "{error:?}");
}

#[test]
fn a_weight_reshapes_the_surface_and_keeps_the_seams() {
    let backend = ExactBackend::new();
    let (low, high, blend) = s_blend(&backend);
    let tangent = LoftSurfaceContinuity::Tangent;
    let weighted = backend
        .move_surface_control_point(
            &blend,
            SurfaceControlPoint {
                u_index: 3,
                v_index: 10,
                offset_mm: [0.0, 0.0, 0.0],
                weight: Some(4.0),
            },
            &[(&low, tangent), (&high, tangent)],
        )
        .expect("weight")
        .body;
    assert_ne!(weighted.result_fingerprint, blend.result_fingerprint);
    for weight in [0.0, -1.0, f64::NAN] {
        let error = backend
            .move_surface_control_point(
                &blend,
                SurfaceControlPoint {
                    u_index: 3,
                    v_index: 10,
                    offset_mm: [0.0, 0.0, 0.0],
                    weight: Some(weight),
                },
                &[],
            )
            .expect_err("weight must be positive");
        assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
    }
}

#[test]
fn a_boundary_surface_moves_with_its_trimmed_edges() {
    let backend = ExactBackend::new();
    let low = plate(&backend, [0.0, 50.0], 0.0);
    let net = backend.surface_control_net(&low).expect("net");
    assert!(net.u_count >= 2 && net.v_count >= 2, "{net:?}");
    let (u_index, v_index) = (net.u_count / 2, net.v_count / 2);
    let moved = backend
        .move_surface_control_point(&low, lift(u_index, v_index, 10.0), &[])
        .expect("lift the middle")
        .body;
    let bounds = moved.topology.bounds_mm;
    assert!(bounds.max.z > 0.1, "{bounds:?}");
    // The plate's straight edges still bound the lifted surface.
    assert!(
        bounds.min.x > -1.0e-3 && bounds.max.x < 50.0 + 1.0e-3,
        "{bounds:?}"
    );
}

#[test]
fn a_control_point_outside_the_net_or_no_change_is_refused() {
    let backend = ExactBackend::new();
    let (_, _, blend) = s_blend(&backend);
    let net = backend.surface_control_net(&blend).expect("net");
    assert_eq!(net.u_count, 6, "{net:?}");
    let error = backend
        .move_surface_control_point(&blend, lift(6, 0, 1.0), &[])
        .expect_err("past the last row");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
    assert!(
        error.diagnostic.contains(&format!("6 x {}", net.v_count)),
        "{error:?}"
    );
    let error = backend
        .move_surface_control_point(&blend, lift(2, 0, 0.0), &[])
        .expect_err("nothing moves");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
}
