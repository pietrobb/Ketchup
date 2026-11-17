//! A blend surface bridges the gap between the facing edges of two surfaces.
//! Each side meets its surface in position (G0), tangent plane (G1) or
//! curvature (G2), and the kernel measures every seam it claims.

use ketchup_exact::{
    ExactBackend, ExactBody, GeometryErrorCode, LoftSurfaceContinuity, SpatialProfileSegment,
};

fn line(start_mm: [f64; 3], end_mm: [f64; 3]) -> SpatialProfileSegment {
    SpatialProfileSegment::Line { start_mm, end_mm }
}

const RADIUS: f64 = 50.0;
const WIDTH: f64 = 50.0;

/// The end of a 60 degree arc of radius 50 about the y axis line through
/// (0, y, 50), starting at the bottom (0, y, 0).
fn arc_end() -> [f64; 3] {
    let angle = -30.0_f64.to_radians();
    [RADIUS * angle.cos(), 0.0, RADIUS + RADIUS * angle.sin()]
}

fn arc(y: f64, reversed: bool) -> SpatialProfileSegment {
    let [x, _, z] = arc_end();
    let (start_mm, end_mm) = if reversed {
        ([x, y, z], [0.0, y, 0.0])
    } else {
        ([0.0, y, 0.0], [x, y, z])
    };
    SpatialProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm: [0.0, y, RADIUS],
        // About +y the bottom turns toward +x clockwise.
        normal: [0.0, 1.0, 0.0],
        clockwise: !reversed,
    }
}

/// A curved patch: two arcs of radius 50 joined by straight edges along y,
/// held to the same arc halfway between them. Its free straight edge at
/// x = 43.3 faces the flat plate.
fn curved_patch(backend: &ExactBackend) -> ExactBody {
    let [x, _, z] = arc_end();
    backend
        .fill_network_surface(
            &[
                arc(0.0, false),
                line([x, 0.0, z], [x, WIDTH, z]),
                arc(WIDTH, true),
                line([0.0, WIDTH, 0.0], [0.0, 0.0, 0.0]),
            ],
            &[arc(WIDTH / 2.0, false)],
        )
        .expect("curved patch")
        .body
}

/// A flat plate at z = 60 from x = 80 to 130.
fn flat_plate(backend: &ExactBackend) -> ExactBody {
    let corners = [
        [80.0, 0.0, 60.0],
        [130.0, 0.0, 60.0],
        [130.0, WIDTH, 60.0],
        [80.0, WIDTH, 60.0],
    ];
    let segments: Vec<_> = (0..4)
        .map(|index| line(corners[index], corners[(index + 1) % 4]))
        .collect();
    backend
        .fill_boundary_surface(&segments)
        .expect("flat plate")
        .body
}

#[test]
fn a_tangent_blend_meets_both_surfaces_with_matching_normals() {
    let backend = ExactBackend::new();
    let curved = curved_patch(&backend);
    let flat = flat_plate(&backend);
    let blend = backend
        .blend_surfaces(
            &curved,
            LoftSurfaceContinuity::Tangent,
            &flat,
            LoftSurfaceContinuity::Tangent,
        )
        .expect("tangent blend")
        .body;
    let topology = &blend.topology;
    assert_eq!((topology.solid_count, topology.face_count), (0, 1));
    assert!(topology.bounds_mm.min.x > 43.0 && topology.bounds_mm.max.x < 80.0 + 1.0e-3);

    for (name, support) in [("curved", &curved), ("flat", &flat)] {
        let seam = backend.measure_seam(&blend, support).expect(name);
        assert!((seam.length_mm - WIDTH).abs() < 1.0e-3, "{name}: {seam:?}");
        assert!(seam.gap_mm < 1.0e-3, "{name}: {seam:?}");
        assert!(seam.normal_angle_rad < 1.0e-3, "{name}: {seam:?}");
    }
}

#[test]
fn a_curvature_blend_also_matches_the_curvature_across_the_seam() {
    let backend = ExactBackend::new();
    let curved = curved_patch(&backend);
    let flat = flat_plate(&backend);
    let blend = |continuity| {
        backend
            .blend_surfaces(&curved, continuity, &flat, continuity)
            .expect("blend")
            .body
    };
    let tangent = backend
        .measure_seam(&blend(LoftSurfaceContinuity::Tangent), &curved)
        .expect("tangent seam");
    let curvature_blend = blend(LoftSurfaceContinuity::Curvature);
    let curvature = backend
        .measure_seam(&curvature_blend, &curved)
        .expect("curvature seam");
    // The patch bends at about 1/50 per mm across its free edge.
    assert!(
        curvature.second_curvature_per_mm.abs() > 0.01,
        "{curvature:?}"
    );
    assert!(curvature.normal_angle_rad < 1.0e-3, "{curvature:?}");
    assert!(
        curvature.curvature_difference_per_mm < 0.05 * curvature.second_curvature_per_mm.abs(),
        "G2 matches the bend: {curvature:?}"
    );
    assert!(
        tangent.curvature_difference_per_mm > 4.0 * curvature.curvature_difference_per_mm,
        "G1 alone does not carry the bend: {tangent:?} vs {curvature:?}"
    );
    let flat_seam = backend
        .measure_seam(&curvature_blend, &flat)
        .expect("flat seam");
    assert!(
        flat_seam.curvature_difference_per_mm < 0.05 * curvature.second_curvature_per_mm.abs(),
        "the blend runs into the plate flat: {flat_seam:?}"
    );
}

#[test]
fn a_position_blend_only_meets_the_edges() {
    let backend = ExactBackend::new();
    let curved = curved_patch(&backend);
    let flat = flat_plate(&backend);
    let blend = backend
        .blend_surfaces(
            &curved,
            LoftSurfaceContinuity::Position,
            &flat,
            LoftSurfaceContinuity::Position,
        )
        .expect("position blend")
        .body;
    let seam = backend.measure_seam(&blend, &flat).expect("seam");
    assert!(seam.gap_mm < 1.0e-3, "{seam:?}");
    // Its sides leave the plate flat, but nothing holds the rest of the
    // seam to the plate's tangent plane.
    assert!(
        seam.normal_angle_rad > 5.0e-3,
        "a G0 blend is not held tangent: {seam:?}"
    );
}

#[test]
fn blending_a_surface_with_itself_or_a_solid_is_refused() {
    let backend = ExactBackend::new();
    let flat = flat_plate(&backend);
    let error = backend
        .blend_surfaces(
            &flat,
            LoftSurfaceContinuity::Tangent,
            &flat,
            LoftSurfaceContinuity::Tangent,
        )
        .expect_err("refuse one surface twice");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");

    let solid = backend
        .make_box(ketchup_exact::BoxSpec {
            origin_mm: ketchup_exact::Point3::ORIGIN,
            size_mm: ketchup_exact::Size3 {
                x: 10.0,
                y: 10.0,
                z: 10.0,
            },
        })
        .expect("box")
        .body;
    let error = backend
        .blend_surfaces(
            &solid,
            LoftSurfaceContinuity::Tangent,
            &flat,
            LoftSurfaceContinuity::Tangent,
        )
        .expect_err("refuse a solid");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
}

#[test]
fn surfaces_without_a_shared_edge_have_no_seam() {
    let backend = ExactBackend::new();
    let error = backend
        .measure_seam(&curved_patch(&backend), &flat_plate(&backend))
        .expect_err("apart surfaces share no edge");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter, "{error:?}");
}
