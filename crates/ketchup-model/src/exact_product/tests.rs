use super::*;

#[test]
fn exact_body_package_keeps_large_graph_storage_indirect() {
    let package_size = std::mem::size_of::<ExactBodyPackage>();
    let graph_size = std::mem::size_of::<ExactBRepGraphPackage>();
    assert!(
        package_size <= 576,
        "package={package_size}, graph={graph_size}"
    );
    assert!(graph_size <= 576);
}

#[test]
fn line_arc_profile_self_contact_uses_the_given_tolerance() {
    // A slot 2 mm long and 0.1 mm wide: its two straight sides are 0.1 mm apart.
    let slot = [
        ProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [2.0, 0.0],
        },
        ProfileSegment::CircularArc {
            start_mm: [2.0, 0.0],
            end_mm: [2.0, 0.1],
            center_mm: [2.0, 0.05],
            clockwise: false,
        },
        ProfileSegment::Line {
            start_mm: [2.0, 0.1],
            end_mm: [0.0, 0.1],
        },
        ProfileSegment::CircularArc {
            start_mm: [0.0, 0.1],
            end_mm: [0.0, 0.0],
            center_mm: [0.0, 0.05],
            clockwise: false,
        },
    ];
    assert!(exact_planar_offset_profile(&slot, true, TolerancePolicy::default()).is_some());
    // With a document tolerance coarser than the slot width the sides touch.
    let coarse = TolerancePolicy::new(0.5).unwrap();
    assert!(exact_planar_offset_profile(&slot, true, coarse).is_none());
}

#[test]
fn segment_contact_tolerance_is_a_distance() {
    // Two parallel 0.1 mm segments 0.05 mm apart do not touch at a 0.01 mm tolerance,
    // however short they are; the tolerance bounds their distance, not a cross product.
    let touch = |gap: f64, tolerance: f64| {
        planar_line_segments_intersect([0.0, 0.0], [0.1, 0.0], [0.0, gap], [0.1, gap], tolerance)
    };
    assert!(!touch(0.05, 0.01));
    assert!(touch(0.005, 0.01));
}

fn line(start: [f64; 2], end: [f64; 2]) -> ExactBRepPlanarSegment {
    ExactBRepPlanarSegment::Line {
        start_bits: start.map(f64::to_bits),
        end_bits: end.map(f64::to_bits),
    }
}

fn circle(center: [f64; 2], radius: f64) -> ExactBRepPlanarLoop {
    ExactBRepPlanarLoop::Circle {
        center_bits: center.map(f64::to_bits),
        radius_bits: radius.to_bits(),
    }
}

fn rectangle(min: [f64; 2], max: [f64; 2]) -> ExactBRepPlanarLoop {
    ExactBRepPlanarLoop::Boundary {
        segments: vec![
            line(min, [max[0], min[1]]),
            line([max[0], min[1]], max),
            line(max, [min[0], max[1]]),
            line([min[0], max[1]], min),
        ],
    }
}

#[test]
fn compound_offset_region_requires_contained_disjoint_non_nested_holes() {
    let valid = ExactPlanarOffsetRegion {
        outer: rectangle([-10.0, -10.0], [10.0, 10.0]),
        holes: vec![circle([-3.0, 0.0], 1.0), circle([3.0, 0.0], 1.0)],
    };
    assert!(valid.has_valid_encoding(0.25));
    assert!(valid.has_valid_encoding(-0.25));

    let outside = ExactPlanarOffsetRegion {
        outer: valid.outer.clone(),
        holes: vec![circle([12.0, 0.0], 1.0)],
    };
    let overlapping = ExactPlanarOffsetRegion {
        outer: valid.outer.clone(),
        holes: vec![circle([0.0, 0.0], 2.0), circle([3.0, 0.0], 2.0)],
    };
    let nested = ExactPlanarOffsetRegion {
        outer: valid.outer,
        holes: vec![circle([0.0, 0.0], 2.0), circle([0.0, 0.0], 1.0)],
    };
    for invalid in [outside, overlapping, nested] {
        assert!(!invalid.has_valid_encoding(0.25));
        assert!(!invalid.has_valid_encoding(-0.25));
    }
}
