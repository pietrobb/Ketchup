use super::*;

/// R1119og-MF-800/803: a drill and a ball end mill sweep their real ends, not a
/// flat cylinder, so the stock they leave is the stock the machine leaves.
#[test]
fn tipped_axial_tools_sweep_their_point_and_ball() {
    use std::f64::consts::PI;
    let backend = ExactBackend::new();
    let (radius, length) = (2.0, 5.0);
    let sweep = |motion: AxialToolMotion, tip: AxialToolTip| {
        backend
            .sweep_axial_tool(AxialToolSweepSpec {
                motion,
                radius_mm: radius,
                axial_length_mm: length,
                tip,
            })
            .unwrap()
    };
    let plunge = AxialToolMotion::Line {
        start_mm: [3.0, 4.0, 0.0],
        end_mm: [3.0, 4.0, -5.0],
    };
    // A drill's point: the shank down to the cone, then the cone a third of its cylinder.
    let point = 1.2;
    let drilled = sweep(plunge, AxialToolTip::Cone { length_mm: point });
    assert_eq!(drilled.body.topology.solid_count, 1);
    assert_close(
        drilled.body.topology.volume_mm3,
        PI * radius * radius * (length + 5.0 - point) + PI * radius * radius * point / 3.0,
    );
    assert_close(drilled.body.topology.bounds_mm.min.z, -5.0);

    // A ball end mill: its section is the shank's rectangle on a half disc.
    let section = 2.0 * radius * (length - radius) + PI * radius * radius / 2.0;
    let tool = PI * radius * radius * (length - radius) + 2.0 / 3.0 * PI * radius.powi(3);
    let line = sweep(
        AxialToolMotion::Line {
            start_mm: [0.0, 0.0, 1.0],
            end_mm: [10.0, 0.0, 1.0],
        },
        AxialToolTip::Ball,
    );
    assert_eq!(line.body.topology.solid_count, 1);
    assert_close(line.body.topology.volume_mm3, section * 10.0 + tool);
    assert_close(line.body.topology.bounds_mm.min.z, 1.0);

    // Revolved a quarter turn about an axis 10 mm off, the section's centroid
    // travels 10 * pi / 2.
    let arc = sweep(
        AxialToolMotion::Arc {
            start_mm: [10.0, 0.0, 2.0],
            end_mm: [0.0, 10.0, 2.0],
            center_mm: [0.0, 0.0, 2.0],
            clockwise: false,
        },
        AxialToolTip::Ball,
    );
    assert_eq!(arc.body.topology.solid_count, 1);
    assert_close(
        arc.body.topology.volume_mm3,
        section * 10.0 * PI / 2.0 + tool,
    );
    assert_close(arc.body.topology.bounds_mm.min.z, 2.0);

    // An arc tighter than the tool still gives one solid, clockwise as well.
    for tip in [AxialToolTip::Ball, AxialToolTip::Cone { length_mm: point }] {
        let tight = sweep(
            AxialToolMotion::Arc {
                start_mm: [1.0, 0.0, 0.0],
                end_mm: [0.0, -1.0, 0.0],
                center_mm: [0.0, 0.0, 0.0],
                clockwise: true,
            },
            tip,
        );
        assert_eq!(tight.body.topology.solid_count, 1);
        assert_close(tight.body.topology.bounds_mm.min.z, 0.0);
    }

    // A ball is as long as the radius, and a point must leave a shank.
    for tip in [
        AxialToolTip::Cone { length_mm: length },
        AxialToolTip::Cone { length_mm: 0.0 },
    ] {
        assert_eq!(
            backend
                .sweep_axial_tool(AxialToolSweepSpec {
                    motion: plunge,
                    radius_mm: radius,
                    axial_length_mm: length,
                    tip,
                })
                .unwrap_err()
                .code,
            GeometryErrorCode::InvalidParameter
        );
    }
}
