//! A guided Loft with a second guide curve (rail): every section across the
//! guide is stretched about the guide so its border meets the rail where the
//! rail crosses its plane, and a rail that does not start on the first section
//! and end on the last is refused.

use ketchup_exact::{
    ExactBackend, FramedLoftProfile, FramedLoftSection, FramedLoftSpec, GeometryErrorCode,
    LoftSurfaceContinuity, PlanarProfileLoop, PlanarProfileSegment, SpatialProfileSegment,
};
use ketchup_geometry::linalg::{Frame, Vec3, cross, length, sub};

const HEIGHT: f64 = 20.0;
const END_RADIUS: f64 = 5.0;
const MIDDLE_RADIUS: f64 = 8.0;

fn ends(profile: PlanarProfileLoop) -> FramedLoftSpec {
    let section = |elevation_mm| FramedLoftSection {
        elevation_mm,
        frame: Frame::WORLD,
        profile: FramedLoftProfile::Planar(profile.clone()),
    };
    FramedLoftSpec {
        sections: vec![section(0.0), section(HEIGHT)],
    }
}

fn circles() -> FramedLoftSpec {
    ends(PlanarProfileLoop::Circle {
        center_mm: [0.0, 0.0],
        radius_mm: END_RADIUS,
    })
}

/// A square of side 2 `END_RADIUS` whose right side starts on the rail.
fn squares() -> FramedLoftSpec {
    let r = END_RADIUS;
    let corners = [[r, 0.0], [r, r], [-r, r], [-r, -r], [r, -r], [r, 0.0]];
    ends(PlanarProfileLoop::Segments(
        corners
            .windows(2)
            .map(|pair| PlanarProfileSegment::Line {
                start_mm: pair[0],
                end_mm: pair[1],
            })
            .collect(),
    ))
}

/// Integral over the height of the stretched section area: the half width at
/// height 20 t is r(t) = 5 + 12 u, u = t - t^2, and the integral of
/// (25 + 120 u + 144 u^2) dt is 49.8.
fn stretched_volume(area_per_squared_half_width: f64) -> f64 {
    HEIGHT * area_per_squared_half_width * 49.8
}

fn assert_volume(volume_mm3: f64, expected: f64) {
    assert!(
        (volume_mm3 - expected).abs() <= 0.002 * expected,
        "volume {volume_mm3} != {expected}"
    );
}

fn axis() -> Vec<SpatialProfileSegment> {
    vec![SpatialProfileSegment::Line {
        start_mm: [0.0, 0.0, 0.0],
        end_mm: [0.0, 0.0, HEIGHT],
    }]
}

/// A cubic Bezier in the XZ plane from the first circle to the last, bulging
/// out to `MIDDLE_RADIUS` half way: r(t) = 5 + 12 t (1 - t), z = 20 t.
fn bulging_rail(start_x: f64) -> Vec<SpatialProfileSegment> {
    let control_x = (8.0 * MIDDLE_RADIUS - 2.0 * END_RADIUS) / 6.0;
    vec![SpatialProfileSegment::CubicBezier {
        start_mm: [start_x, 0.0, 0.0],
        control_1_mm: [control_x, 0.0, HEIGHT / 3.0],
        control_2_mm: [control_x, 0.0, 2.0 * HEIGHT / 3.0],
        end_mm: [END_RADIUS, 0.0, HEIGHT],
    }]
}

#[test]
fn sections_stretch_to_keep_their_border_on_the_rail() {
    let backend = ExactBackend::new();
    let barrel = backend
        .loft_framed_with_rail(
            &circles(),
            &axis(),
            &bulging_rail(END_RADIUS),
            LoftSurfaceContinuity::Position,
            true,
        )
        .expect("loft along the axis with the rail");
    let topology = &barrel.body.topology;
    assert_eq!(topology.solid_count, 1);
    assert_volume(topology.volume_mm3, stretched_volume(std::f64::consts::PI));
    assert!(
        (topology.bounds_mm.max.x - MIDDLE_RADIUS).abs() <= 0.01,
        "barrel reaches x = {}",
        topology.bounds_mm.max.x
    );

    let without_rail = backend
        .loft_framed_profiles_with_controls(
            &circles(),
            Some(&axis()),
            LoftSurfaceContinuity::Position,
        )
        .expect("loft along the axis alone");
    assert!(without_rail.body.topology.volume_mm3 < 0.6 * topology.volume_mm3);
}

#[test]
fn a_square_section_stretches_to_the_rail_as_well() {
    let backend = ExactBackend::new();
    let body = backend
        .loft_framed_with_rail(
            &squares(),
            &axis(),
            &bulging_rail(END_RADIUS),
            LoftSurfaceContinuity::Position,
            true,
        )
        .expect("square loft along the axis with the rail");
    assert_eq!(body.body.topology.solid_count, 1);
    assert_volume(body.body.topology.volume_mm3, stretched_volume(4.0));
}

#[test]
fn a_surface_loft_follows_the_rail_too() {
    let backend = ExactBackend::new();
    let surface = backend
        .loft_framed_with_rail(
            &circles(),
            &axis(),
            &bulging_rail(END_RADIUS),
            LoftSurfaceContinuity::Position,
            false,
        )
        .expect("surface loft with the rail");
    let topology = &surface.body.topology;
    assert_eq!(topology.solid_count, 0);
    assert!((topology.bounds_mm.max.x - MIDDLE_RADIUS).abs() <= 0.05);
    // Its display mesh covers the surface once: the area of the triangles is the
    // exact area, as the model checks for every surface a worker returns.
    let exact_area: f64 = topology.faces.iter().map(|face| face.area_mm2).sum();
    let mesh = backend
        .tessellate_body(&surface.body, 0.05, 0.5, 1_000_000)
        .expect("tessellated surface");
    let mesh_area: f64 = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle
                .vertex_indices
                .map(|index| mesh.vertices_mm[index as usize]);
            0.5 * length(cross(sub(b, a), sub(c, a)))
        })
        .sum();
    assert!(
        (mesh_area - exact_area).abs() <= 0.01 * exact_area,
        "mesh area {mesh_area} != exact area {exact_area}"
    );
}

/// A U in the XZ plane, two mirrored cubic halves: up from x = 15 - `half`,
/// over the top at z = `top` with x = 15 and down to x = 15 + `half`.
fn u_bend(half: f64, top: f64) -> Vec<SpatialProfileSegment> {
    let at = |dx: f64, z: f64| [15.0 + dx, 0.0, z];
    let up = 2.0 * top / 3.0;
    let flat = half / 3.0;
    vec![
        SpatialProfileSegment::CubicBezier {
            start_mm: at(-half, 0.0),
            control_1_mm: at(-half, up),
            control_2_mm: at(-flat, top),
            end_mm: at(0.0, top),
        },
        SpatialProfileSegment::CubicBezier {
            start_mm: at(0.0, top),
            control_1_mm: at(flat, top),
            control_2_mm: at(half, up),
            end_mm: at(half, 0.0),
        },
    ]
}

#[test]
fn a_rail_along_a_u_shaped_guide_takes_the_crossing_of_its_own_leg() {
    // Every station plane on one leg crosses the other leg of the guide and of
    // the rail too (R1120ac-EXA-903); the station's crossing is the near one.
    let circle = FramedLoftProfile::Planar(PlanarProfileLoop::Circle {
        center_mm: [0.0, 0.0],
        radius_mm: END_RADIUS,
    });
    // The second section faces down at x = 30, turned half about y as the
    // guide turns it, 1 mm along its normal from its frame's origin.
    let down = Frame {
        origin: Vec3::new(30.0, 0.0, 1.0),
        x: -Vec3::X,
        y: Vec3::Y,
        z: -Vec3::Z,
    };
    let spec = FramedLoftSpec {
        sections: vec![
            FramedLoftSection {
                elevation_mm: 0.0,
                frame: Frame::WORLD,
                profile: circle.clone(),
            },
            FramedLoftSection {
                elevation_mm: 1.0,
                frame: down,
                profile: circle,
            },
        ],
    };
    // The guide tops out at z = 30 and the rail at z = 36, so the section at
    // the top is stretched from radius 5 to 6.
    let body = ExactBackend::new()
        .loft_framed_with_rail(
            &spec,
            &u_bend(15.0, 30.0),
            &u_bend(15.0 + END_RADIUS, 36.0),
            LoftSurfaceContinuity::Position,
            true,
        )
        .expect("loft along the U with a rail beside it");
    let topology = &body.body.topology;
    assert_eq!(topology.solid_count, 1);
    assert!(
        (topology.bounds_mm.max.z - 36.0).abs() <= 0.05,
        "the bend reaches z = {}",
        topology.bounds_mm.max.z
    );
}

#[test]
fn a_rail_off_the_sections_is_refused() {
    let backend = ExactBackend::new();
    let error = backend
        .loft_framed_with_rail(
            &circles(),
            &axis(),
            &bulging_rail(END_RADIUS + 3.0),
            LoftSurfaceContinuity::Position,
            true,
        )
        .expect_err("a rail that starts away from the first section");
    assert_eq!(error.code, GeometryErrorCode::InvalidParameter);
}
