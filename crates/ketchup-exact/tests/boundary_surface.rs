//! A surface filled into a closed loop of boundary curves: it meets every
//! boundary curve, a flat boundary fills to its exact area, and a loop that is
//! not closed is refused before the kernel runs.

use ketchup_exact::{
    ExactBackend, ExactOpOutput, GeometryErrorCode, ShellDirection, SpatialProfileSegment,
};

fn line(start_mm: [f64; 3], end_mm: [f64; 3]) -> SpatialProfileSegment {
    SpatialProfileSegment::Line { start_mm, end_mm }
}

fn polygon(corners: &[[f64; 3]]) -> Vec<SpatialProfileSegment> {
    (0..corners.len())
        .map(|index| line(corners[index], corners[(index + 1) % corners.len()]))
        .collect()
}

fn assert_relative(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{actual} != {expected} within {relative}"
    );
}

/// A one-sided wall of the given thickness on the surface: its volume is the
/// surface area times the thickness.
fn area_by_thickening(backend: &ExactBackend, surface: &ExactOpOutput) -> f64 {
    let wall = backend
        .thicken_surface(&surface.body, 1.0, ShellDirection::Outward)
        .expect("thicken the filled surface");
    assert_eq!(wall.body.topology.solid_count, 1);
    wall.body.topology.volume_mm3
}

#[test]
fn a_flat_rectangle_fills_to_its_area() {
    let backend = ExactBackend::new();
    let surface = backend
        .fill_boundary_surface(&polygon(&[
            [0.0, 0.0, 3.0],
            [20.0, 0.0, 3.0],
            [20.0, 10.0, 3.0],
            [0.0, 10.0, 3.0],
        ]))
        .expect("fill a rectangle");
    let topology = &surface.body.topology;
    assert_eq!((topology.solid_count, topology.face_count), (0, 1));
    assert_eq!(topology.edge_count, 4);
    assert_relative(
        topology.bounds_mm.max.x - topology.bounds_mm.min.x,
        20.0,
        1.0e-9,
    );
    assert_relative(
        topology.bounds_mm.max.y - topology.bounds_mm.min.y,
        10.0,
        1.0e-9,
    );
    assert_relative(area_by_thickening(&backend, &surface), 200.0, 1.0e-6);
}

#[test]
fn a_circle_of_two_spatial_arcs_fills_to_its_disc() {
    let backend = ExactBackend::new();
    let arc = |start_mm, end_mm| SpatialProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm: [0.0, 0.0, 0.0],
        normal: [0.0, 0.0, 1.0],
        clockwise: false,
    };
    let surface = backend
        .fill_boundary_surface(&[
            arc([10.0, 0.0, 0.0], [-10.0, 0.0, 0.0]),
            arc([-10.0, 0.0, 0.0], [10.0, 0.0, 0.0]),
        ])
        .expect("fill a circle");
    assert_eq!(surface.body.topology.face_count, 1);
    assert_relative(
        area_by_thickening(&backend, &surface),
        100.0 * std::f64::consts::PI,
        1.0e-6,
    );
}

#[test]
fn a_twisted_quadrilateral_fills_between_its_edges() {
    let backend = ExactBackend::new();
    let surface = backend
        .fill_boundary_surface(&polygon(&[
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 5.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 5.0],
        ]))
        .expect("fill a twisted quadrilateral");
    let topology = &surface.body.topology;
    assert_eq!((topology.solid_count, topology.face_count), (0, 1));
    let bounds = topology.bounds_mm;
    let slack = 1.0e-3;
    assert!(bounds.min.x >= -slack && bounds.max.x <= 10.0 + slack);
    assert!(bounds.min.y >= -slack && bounds.max.y <= 10.0 + slack);
    assert!(
        bounds.min.z >= -slack && bounds.max.z <= 5.0 + slack,
        "{bounds:?}"
    );
    // Not flat: larger than its 10 x 10 shadow, smaller than the two flat
    // triangles over its diagonal.
    let area = area_by_thickening(&backend, &surface);
    let triangles = 2.0 * 0.5 * 10.0 * (100.0_f64 + 25.0).sqrt();
    assert!(area > 100.0 && area < triangles + 1.0, "area {area}");
}

#[test]
fn a_boundary_that_is_not_one_closed_loop_is_refused() {
    let backend = ExactBackend::new();
    let open = vec![
        line([0.0, 0.0, 0.0], [10.0, 0.0, 0.0]),
        line([10.0, 0.0, 0.0], [10.0, 10.0, 0.0]),
        line([10.0, 10.0, 0.0], [0.0, 10.0, 0.0]),
    ];
    let disconnected = vec![
        line([0.0, 0.0, 0.0], [10.0, 0.0, 0.0]),
        line([10.0, 1.0, 0.0], [10.0, 10.0, 0.0]),
        line([10.0, 10.0, 0.0], [0.0, 0.0, 0.0]),
    ];
    let single = vec![line([0.0, 0.0, 0.0], [10.0, 0.0, 0.0])];
    for boundary in [open, disconnected, single] {
        let error = backend
            .fill_boundary_surface(&boundary)
            .expect_err("refuse a boundary that is not one closed loop");
        assert_eq!(error.code, GeometryErrorCode::InvalidProfile, "{error:?}");
    }
}
