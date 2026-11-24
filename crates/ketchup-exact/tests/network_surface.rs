//! A surface through a network of curves: a closed boundary loop plus curves
//! inside it that the surface must also pass through. The kernel measures
//! how far every curve lies from the surface and refuses a network that no
//! surface meets within the linear tolerance.

use ketchup_exact::{ExactBackend, GeometryErrorCode, SpatialProfileSegment};

fn line(start_mm: [f64; 3], end_mm: [f64; 3]) -> SpatialProfileSegment {
    SpatialProfileSegment::Line { start_mm, end_mm }
}

fn square(side: f64) -> Vec<SpatialProfileSegment> {
    let corners = [
        [0.0, 0.0, 0.0],
        [side, 0.0, 0.0],
        [side, side, 0.0],
        [0.0, side, 0.0],
    ];
    (0..4)
        .map(|index| line(corners[index], corners[(index + 1) % 4]))
        .collect()
}

/// A cubic Bezier from `start` to `end` that rises to `peak` above the
/// ground at its middle (a symmetric cubic reaches 3/4 of its control
/// height there).
fn arch(start_mm: [f64; 3], end_mm: [f64; 3], peak: f64) -> SpatialProfileSegment {
    let lift = peak * 4.0 / 3.0;
    let at = |t: f64| {
        [
            start_mm[0] + (end_mm[0] - start_mm[0]) * t,
            start_mm[1] + (end_mm[1] - start_mm[1]) * t,
            lift,
        ]
    };
    SpatialProfileSegment::CubicBezier {
        start_mm,
        control_1_mm: at(0.25),
        control_2_mm: at(0.75),
        end_mm,
    }
}

#[test]
fn two_crossing_arches_lift_a_flat_boundary_to_their_peak() {
    let backend = ExactBackend::new();
    let flat = backend
        .fill_network_surface(&square(100.0), &[])
        .expect("fill the bare boundary");
    assert!(flat.body.topology.bounds_mm.max.z.abs() < 1.0e-6);

    let arches = [
        arch([0.0, 50.0, 0.0], [100.0, 50.0, 0.0], 20.0),
        arch([50.0, 0.0, 0.0], [50.0, 100.0, 0.0], 20.0),
    ];
    let dome = backend
        .fill_network_surface(&square(100.0), &arches)
        .expect("fill the boundary through both arches");
    let topology = &dome.body.topology;
    assert_eq!((topology.solid_count, topology.face_count), (0, 1));
    assert_eq!(topology.edge_count, 4, "inner curves are not edges");
    let bounds = topology.bounds_mm;
    assert!(
        (bounds.max.z - 20.0).abs() < 0.05,
        "the surface peaks where the arches cross: {bounds:?}"
    );
    assert!(bounds.min.z > -0.05, "{bounds:?}");
    assert!(bounds.min.x > -1.0e-3 && bounds.max.x < 100.0 + 1.0e-3);
}

#[test]
fn a_network_no_surface_can_pass_through_is_refused() {
    let backend = ExactBackend::new();
    // The arches cross above the same point at +20 and -20 mm.
    let crossed = [
        arch([0.0, 50.0, 0.0], [100.0, 50.0, 0.0], 20.0),
        arch([50.0, 0.0, 0.0], [50.0, 100.0, 0.0], -20.0),
    ];
    let error = backend
        .fill_network_surface(&square(100.0), &crossed)
        .expect_err("refuse an inconsistent network");
    assert_eq!(error.code, GeometryErrorCode::InvalidShape, "{error:?}");
    assert!(
        error.diagnostic.contains("misses"),
        "the refusal says how far the surface misses: {error:?}"
    );
}

#[test]
fn too_many_inner_curves_are_refused_before_the_kernel_runs() {
    let backend = ExactBackend::new();
    let many: Vec<_> = (0..65)
        .map(|index| {
            let x = 1.0 + f64::from(index) * 1.5;
            line([x, 0.0, 0.0], [x, 100.0, 0.0])
        })
        .collect();
    let error = backend
        .fill_network_surface(&square(100.0), &many)
        .expect_err("refuse more than 64 inner curves");
    assert_eq!(error.code, GeometryErrorCode::InvalidProfile, "{error:?}");
}
