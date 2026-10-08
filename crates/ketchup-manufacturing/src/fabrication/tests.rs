use super::*;

fn test_stock_geometry(dimensions: PieceDimensions) -> GeneralMachiningGeometry {
    let points = [
        [0.0, 0.0],
        [dimensions.length_mm, 0.0],
        [dimensions.length_mm, dimensions.width_mm],
        [0.0, dimensions.width_mm],
    ];
    GeneralMachiningGeometry::TimberStock {
        frame: identity_machining_frame(),
        cross_section: (0..4)
            .map(|i| GeneralMachiningSegment::Line {
                start_mm: points[i],
                end_mm: points[(i + 1) % 4],
            })
            .collect(),
        start_mm: [0.0; 3],
        length_axis: [0.0, 0.0, 1.0],
        length_mm: dimensions.height_mm,
        cross_section_width_mm: dimensions.length_mm,
        cross_section_height_mm: dimensions.width_mm,
    }
}

#[test]
fn btlx_numeric_values_follow_supported_xsd_ranges() {
    assert_eq!(btlx_component_identifiers(1, 0), Some((1, 1)));
    assert_eq!(
        btlx_component_identifiers(i32::MAX as usize, 0),
        Some((i32::MAX, 1))
    );
    assert_eq!(btlx_component_identifiers(i32::MAX as usize + 1, 0), None);
    let last_index = u32::MAX as usize - 1;
    assert_eq!(
        btlx_component_identifiers(1, last_index),
        Some((1, u32::MAX))
    );
    assert_eq!(btlx_component_identifiers(1, last_index + 1), None);
    assert_eq!(
        format_btlx_positive_number(1.0e-9),
        Some("0.000000001".to_owned())
    );
    assert_eq!(format_btlx_positive_number(1.0e-10), None);
    assert_eq!(format_btlx_positive_number(f64::NAN), None);
    assert_eq!(format_btlx_positive_number(f64::INFINITY), None);
}

#[test]
fn btlx_stock_dimensions_require_a_closed_axis_aligned_rectangle() {
    let valid = test_stock_geometry(PieceDimensions {
        length_mm: 100.0,
        width_mm: 50.0,
        height_mm: 1000.0,
    });
    assert_eq!(
        rectangular_timber_stock_dimensions(&valid),
        Some((1000.0, 100.0, 50.0))
    );

    let mut translated = valid.clone();
    let GeneralMachiningGeometry::TimberStock { start_mm, .. } = &mut translated else {
        unreachable!()
    };
    *start_mm = [10.0, 0.0, 0.0];
    assert_eq!(rectangular_timber_stock_dimensions(&translated), None);

    let mut rotated = valid.clone();
    let GeneralMachiningGeometry::TimberStock { frame, .. } = &mut rotated else {
        unreachable!()
    };
    frame.x_axis = [0.0, 1.0, 0.0];
    frame.y_axis = [-1.0, 0.0, 0.0];
    assert_eq!(rectangular_timber_stock_dimensions(&rotated), None);

    let mut open = valid.clone();
    let GeneralMachiningGeometry::TimberStock { cross_section, .. } = &mut open else {
        unreachable!()
    };
    let GeneralMachiningSegment::Line { end_mm, .. } = &mut cross_section[3] else {
        unreachable!()
    };
    *end_mm = [1.0, 0.0];
    assert_eq!(rectangular_timber_stock_dimensions(&open), None);

    let mut diagonal = valid;
    let GeneralMachiningGeometry::TimberStock { cross_section, .. } = &mut diagonal else {
        unreachable!()
    };
    let GeneralMachiningSegment::Line { end_mm, .. } = &mut cross_section[0] else {
        unreachable!()
    };
    *end_mm = [100.0, 1.0];
    assert_eq!(rectangular_timber_stock_dimensions(&diagonal), None);
}

#[test]
fn woodwop_frame_of_an_odd_axis_order_stays_right_handed() {
    // A cabinet side 19 x 400 x 600: machine X, Y, Z = definition z, y, x,
    // an odd permutation, so machine Y must run from the far edge.
    let side = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
        length_mm: 19.0,
        width_mm: 400.0,
        height_mm: 600.0,
    }))
    .unwrap();
    assert_eq!(side.definition_axes, [2, 1, 0]);
    assert!(side.reversed_y);
    let drill = GeneralMachiningGeometry::CircularDrill {
        frame: GeneralMachiningFrame {
            origin_mm: [9.5, 0.0, 100.0],
            x_axis: [0.0, 0.0, 1.0],
            y_axis: [1.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
        },
        center_mm: [0.0, 0.0],
        diameter_mm: 8.0,
        through: false,
        start_mm: 0.0,
        end_mm: 30.0,
    };
    assert!(
        woodwop_drilling_macro(&drill, side)
            .unwrap()
            .starts_with("<103 \\BohrHoriz\\\nXA=\"100\"\nYA=\"400\"\nZA=\"9.5\"\nBM=\"YM\"\n")
    );
    // Every definition-to-machine map is a proper rotation plus translation.
    for dimensions in [
        [100.0, 50.0, 1000.0],
        [19.0, 400.0, 600.0],
        [600.0, 19.0, 400.0],
    ] {
        let frame = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
            length_mm: dimensions[0],
            width_mm: dimensions[1],
            height_mm: dimensions[2],
        }))
        .unwrap();
        let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            .map(|axis| woodwop_direction(axis, frame));
        assert_eq!(
            ketchup_geometry::linalg::Mat3::from_columns(axes).determinant(),
            1.0,
            "{dimensions:?}"
        );
    }
}

#[test]
fn woodwop_drilling_maps_horizontal_and_top_vertical_axes_and_rejects_unsafe_axes() {
    let beam_stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
        length_mm: 100.0,
        width_mm: 50.0,
        height_mm: 1000.0,
    }))
    .unwrap();
    assert_eq!(beam_stock.definition_axes, [2, 0, 1]);
    assert_eq!(beam_stock.dimensions_mm, [1000.0, 100.0, 50.0]);
    let horizontal = GeneralMachiningGeometry::CircularDrill {
        frame: identity_machining_frame(),
        center_mm: [50.0, 25.0],
        diameter_mm: 10.0,
        through: false,
        start_mm: 0.0,
        end_mm: 50.0,
    };
    let horizontal_macro = woodwop_drilling_macro(&horizontal, beam_stock).unwrap();
    assert!(horizontal_macro.starts_with(
        "<103 \\BohrHoriz\\\nXA=\"0\"\nYA=\"50\"\nZA=\"25\"\nBM=\"XP\"\nTI=\"50\"\nDU=\"10\"\n"
    ));

    let panel_stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
        length_mm: 600.0,
        width_mm: 400.0,
        height_mm: 19.0,
    }))
    .unwrap();
    assert_eq!(panel_stock.definition_axes, [0, 1, 2]);
    assert_eq!(panel_stock.dimensions_mm, [600.0, 400.0, 19.0]);
    let vertical = GeneralMachiningGeometry::CircularDrill {
        frame: GeneralMachiningFrame {
            origin_mm: [0.0, 0.0, 19.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [1.0, 0.0, 0.0],
            normal: [0.0, 0.0, -1.0],
        },
        center_mm: [25.0, 50.0],
        diameter_mm: 5.0,
        through: false,
        start_mm: 0.0,
        end_mm: 12.0,
    };
    let vertical_macro = woodwop_drilling_macro(&vertical, panel_stock).unwrap();
    assert!(
        vertical_macro.starts_with(
            "<102 \\BohrVert\\\nXA=\"50\"\nYA=\"25\"\nBM=\"LS\"\nTI=\"12\"\nDU=\"5\"\n"
        )
    );
    let mut through = vertical.clone();
    let GeneralMachiningGeometry::CircularDrill { end_mm, .. } = &mut through else {
        unreachable!()
    };
    *end_mm = 19.0;
    assert_eq!(woodwop_drilling_macro(&through, panel_stock), None);

    let root_half = 0.5_f64.sqrt();
    let angled = GeneralMachiningGeometry::CircularDrill {
        frame: GeneralMachiningFrame {
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, root_half, root_half],
            normal: [0.0, -root_half, root_half],
        },
        center_mm: [25.0, 25.0],
        diameter_mm: 5.0,
        through: false,
        start_mm: 0.0,
        end_mm: 12.0,
    };
    assert_eq!(woodwop_drilling_macro(&angled, panel_stock), None);

    let from_below = GeneralMachiningGeometry::CircularDrill {
        frame: identity_machining_frame(),
        center_mm: [50.0, 25.0],
        diameter_mm: 5.0,
        through: false,
        start_mm: 0.0,
        end_mm: 12.0,
    };
    assert_eq!(woodwop_drilling_macro(&from_below, panel_stock), None);
}

#[test]
fn woodwop_vertical_pocket_requires_top_rectangular_geometry_and_explicit_tool() {
    let stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
        length_mm: 600.0,
        width_mm: 400.0,
        height_mm: 19.0,
    }))
    .unwrap();
    let pocket = GeneralMachiningGeometry::ProfileCut {
        frame: GeneralMachiningFrame {
            origin_mm: [0.0, 0.0, 19.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [1.0, 0.0, 0.0],
            normal: [0.0, 0.0, -1.0],
        },
        segments: vec![
            GeneralMachiningSegment::Line {
                start_mm: [100.0, 200.0],
                end_mm: [140.0, 200.0],
            },
            GeneralMachiningSegment::Line {
                start_mm: [140.0, 200.0],
                end_mm: [140.0, 260.0],
            },
            GeneralMachiningSegment::Line {
                start_mm: [140.0, 260.0],
                end_mm: [100.0, 260.0],
            },
            GeneralMachiningSegment::Line {
                start_mm: [100.0, 260.0],
                end_mm: [100.0, 200.0],
            },
        ],
        start_mm: 0.0,
        end_mm: 12.0,
    };

    let macro_text = woodwop_vertical_pocket_macro(&pocket, stock, 101).unwrap();
    assert!(macro_text.starts_with(
        "<112 \\Tasche\\\nXA=\"230\"\nYA=\"120\"\nLA=\"60\"\nBR=\"40\"\nRD=\"0\"\nWI=\"0\"\nTI=\"12\"\n"
    ));
    assert!(macro_text.contains("T_=\"101\"\nF_=\"STANDARD\"\n"));
    assert_eq!(woodwop_vertical_pocket_macro(&pocket, stock, 0), None);

    let mut through = pocket.clone();
    let GeneralMachiningGeometry::ProfileCut { end_mm, .. } = &mut through else {
        unreachable!()
    };
    *end_mm = 19.0;
    assert_eq!(woodwop_vertical_pocket_macro(&through, stock, 101), None);

    let mut open = pocket;
    let GeneralMachiningGeometry::ProfileCut { segments, .. } = &mut open else {
        unreachable!()
    };
    let GeneralMachiningSegment::Line { end_mm, .. } = &mut segments[3] else {
        unreachable!()
    };
    *end_mm = [110.0, 200.0];
    assert_eq!(woodwop_vertical_pocket_macro(&open, stock, 101), None);
}

#[test]
fn homag_pin_macro_and_code128_label_share_the_machine_program_identity() {
    let stock = woodwop_stock_frame(&test_stock_geometry(PieceDimensions {
        length_mm: 600.0,
        width_mm: 400.0,
        height_mm: 19.0,
    }))
    .unwrap();
    let hole = PinHole {
        stable_hole_id: "pin-7/0/first".to_owned(),
        instance_path: InstancePath::root(OccurrenceId(1)),
        entry_local_mm: [50.0, 20.0, 19.0],
        inward_unit_local: [0.0, 0.0, -1.0],
        diameter_mm: 8.0,
        depth_mm: 16.0,
        shared_center_world_mm: [50.0, 20.0, 19.0],
    };
    let drilling = woodwop_pin_macro(&hole, stock).unwrap();
    assert!(
        drilling.starts_with(
            "<102 \\BohrVert\\\nXA=\"50\"\nYA=\"20\"\nBM=\"LS\"\nTI=\"16\"\nDU=\"8\"\n"
        )
    );
    let mpr = woodwop_mpr_output(stock, &[drilling], HOMAG_BHX_PRODUCTION_PACKAGE_V1).unwrap();
    let mpr = String::from_utf8(mpr).unwrap();
    assert!(mpr.contains("\\ketchup.homag-bhx-production-package.v1\\"));
    assert_eq!(mpr.matches("\\BohrVert\\").count(), 1);

    let label = homag_code128_svg("ABCDEF123456").unwrap();
    let label_again = homag_code128_svg("ABCDEF123456").unwrap();
    assert_eq!(label, label_again);
    let label = String::from_utf8(label).unwrap();
    assert!(label.contains(HOMAG_CODE128_LABEL_V1));
    assert!(label.contains(">ABCDEF123456</text>"));
    assert!(label.contains("aria-label=\"HOMAG program ABCDEF123456\""));
    assert!(homag_code128_svg("SHORT").is_none());
    assert!(homag_code128_svg("12345678901!").is_none());
}

#[test]
fn btlx_drilling_maps_definition_axes_and_rejects_invalid_geometry() {
    let valid = GeneralMachiningGeometry::CircularDrill {
        frame: identity_machining_frame(),
        center_mm: [50.0, 25.0],
        diameter_mm: 10.0,
        through: false,
        start_mm: 10.0,
        end_mm: 60.0,
    };
    let drilling = btlx_drilling(&valid, [100.0, 50.0, 1000.0]).unwrap();
    assert_eq!(drilling.reference_point_mm, [10.0, 0.0, 0.0]);
    assert_eq!(drilling.x_vector, [0.0, 1.0, 0.0]);
    assert_eq!(drilling.y_vector, [0.0, 0.0, 1.0]);
    assert_eq!(
        (
            drilling.start_x.as_str(),
            drilling.start_y.as_str(),
            drilling.depth.as_str(),
            drilling.diameter.as_str(),
        ),
        ("50", "25", "50", "10")
    );

    let mut zero_depth = valid.clone();
    let GeneralMachiningGeometry::CircularDrill {
        start_mm, end_mm, ..
    } = &mut zero_depth
    else {
        unreachable!()
    };
    *end_mm = *start_mm;
    assert_eq!(btlx_drilling(&zero_depth, [100.0, 50.0, 1000.0]), None);

    let mut invalid_diameter = valid.clone();
    let GeneralMachiningGeometry::CircularDrill { diameter_mm, .. } = &mut invalid_diameter else {
        unreachable!()
    };
    *diameter_mm = 50_000.000_000_001;
    assert_eq!(
        btlx_drilling(&invalid_diameter, [100.0, 50.0, 1000.0]),
        None
    );

    let mut invalid_center = valid.clone();
    let GeneralMachiningGeometry::CircularDrill { center_mm, .. } = &mut invalid_center else {
        unreachable!()
    };
    *center_mm = [100_000.000_000_001, 25.0];
    assert_eq!(btlx_drilling(&invalid_center, [100.0, 50.0, 1000.0]), None);

    let mut invalid_frame = valid;
    let GeneralMachiningGeometry::CircularDrill { frame, .. } = &mut invalid_frame else {
        unreachable!()
    };
    frame.x_axis = [2.0, 0.0, 0.0];
    assert_eq!(btlx_drilling(&invalid_frame, [100.0, 50.0, 1000.0]), None);
}

#[test]
fn btlx_blind_depth_uses_entry_and_direction_in_stock_coordinates() {
    // X-directed drilling must use the 100 mm stock extent, not its 18 mm
    // thickness. A shifted entry has only the remaining stock available.
    let mut drill = GeneralMachiningGeometry::CircularDrill {
        frame: GeneralMachiningFrame {
            origin_mm: [0.0, 50.0, 9.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
            normal: [1.0, 0.0, 0.0],
        },
        center_mm: [0.0, 0.0],
        diameter_mm: 8.0,
        through: false,
        start_mm: 0.0,
        end_mm: 50.0,
    };
    let dimensions = [100.0, 100.0, 18.0];
    assert_eq!(btlx_drilling(&drill, dimensions).unwrap().depth, "50");
    for depth in [100.0, 122.0] {
        let GeneralMachiningGeometry::CircularDrill { end_mm, .. } = &mut drill else {
            panic!("expected drill")
        };
        *end_mm = depth;
        assert!(btlx_drilling(&drill, dimensions).is_none());
    }
    let GeneralMachiningGeometry::CircularDrill {
        start_mm, end_mm, ..
    } = &mut drill
    else {
        panic!("expected drill")
    };
    *start_mm = 75.0;
    *end_mm = 110.0;
    assert!(btlx_drilling(&drill, dimensions).is_none());
    // An angled ray exits the top before traversing the full X extent.
    let GeneralMachiningGeometry::CircularDrill {
        frame,
        start_mm,
        end_mm,
        ..
    } = &mut drill
    else {
        panic!("expected drill")
    };
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    frame.y_axis = [-diagonal, 0.0, diagonal];
    frame.normal = [diagonal, 0.0, diagonal];
    *start_mm = 0.0;
    *end_mm = 6.0;
    assert_eq!(btlx_drilling(&drill, dimensions).unwrap().depth, "6");
    let GeneralMachiningGeometry::CircularDrill { end_mm, .. } = &mut drill else {
        panic!("expected drill")
    };
    *end_mm = 10.0;
    assert!(btlx_drilling(&drill, dimensions).is_none());
}

#[test]
fn btlx_free_contour_requires_a_closed_simple_line_arc_profile() {
    let line = |start_mm, end_mm| GeneralMachiningSegment::Line { start_mm, end_mm };
    let valid = GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments: vec![
            line([0.0, 0.0], [20.0, 0.0]),
            line([20.0, 0.0], [20.0, 10.0]),
            line([20.0, 10.0], [0.0, 10.0]),
            line([0.0, 10.0], [0.0, 0.0]),
        ],
        start_mm: 10.0,
        end_mm: 30.0,
    };
    let contour = btlx_free_contour(&valid).unwrap();
    assert_eq!(contour.reference_point_mm, [10.0, 0.0, 0.0]);
    assert_eq!(contour.tool_position, "left");
    assert_eq!(contour.start_point, ["0".to_owned(), "0".to_owned()]);
    assert_eq!(contour.depth, "20");

    let clockwise = GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments: vec![
            line([0.0, 0.0], [0.0, 10.0]),
            line([0.0, 10.0], [20.0, 10.0]),
            line([20.0, 10.0], [20.0, 0.0]),
            line([20.0, 0.0], [0.0, 0.0]),
        ],
        start_mm: 0.0,
        end_mm: 20.0,
    };
    assert_eq!(
        btlx_free_contour(&clockwise).unwrap().tool_position,
        "right"
    );

    let mut open = valid.clone();
    let GeneralMachiningGeometry::ProfileCut { segments, .. } = &mut open else {
        unreachable!()
    };
    let GeneralMachiningSegment::Line { end_mm, .. } = &mut segments[3] else {
        unreachable!()
    };
    *end_mm = [1.0, 0.0];
    assert_eq!(btlx_free_contour(&open), None);

    let irregular = GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments: vec![
            line([0.0, 0.0], [20.0, 0.0]),
            line([20.0, 0.0], [25.0, 5.0]),
            line([25.0, 5.0], [10.0, 15.0]),
            line([10.0, 15.0], [0.0, 10.0]),
            line([0.0, 10.0], [0.0, 0.0]),
        ],
        start_mm: 0.0,
        end_mm: 20.0,
    };
    assert_eq!(btlx_free_contour(&irregular).unwrap().segments.len(), 5);

    let arc_profile = GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments: vec![
            line([10.0, 10.0], [30.0, 10.0]),
            GeneralMachiningSegment::CircularArc {
                start_mm: [30.0, 10.0],
                end_mm: [30.0, 30.0],
                center_mm: [30.0, 20.0],
                clockwise: false,
            },
            line([30.0, 30.0], [10.0, 30.0]),
            line([10.0, 30.0], [10.0, 10.0]),
        ],
        start_mm: 0.0,
        end_mm: 18.0,
    };
    assert!(matches!(
        &btlx_free_contour(&arc_profile).unwrap().segments[1],
        BtlxContourSegment::Arc {
            end_point,
            point_on_arc,
        } if end_point == &["30".to_owned(), "30".to_owned()]
            && point_on_arc == &["40".to_owned(), "20".to_owned()]
    ));
    let mut mismatched_arc_radius = arc_profile;
    let GeneralMachiningGeometry::ProfileCut { segments, .. } = &mut mismatched_arc_radius else {
        unreachable!()
    };
    let GeneralMachiningSegment::CircularArc { center_mm, .. } = &mut segments[1] else {
        unreachable!()
    };
    *center_mm = [31.0, 19.0];
    assert_eq!(btlx_free_contour(&mismatched_arc_radius), None);

    let self_intersecting = GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments: vec![
            line([0.0, 0.0], [20.0, 20.0]),
            line([20.0, 20.0], [0.0, 20.0]),
            line([0.0, 20.0], [20.0, 0.0]),
            line([20.0, 0.0], [0.0, 0.0]),
        ],
        start_mm: 0.0,
        end_mm: 20.0,
    };
    assert_eq!(btlx_free_contour(&self_intersecting), None);

    let collapsed_after_formatting = GeneralMachiningGeometry::ProfileCut {
        frame: identity_machining_frame(),
        segments: vec![
            line([0.0, 0.0], [1.0e-10, 0.0]),
            line([1.0e-10, 0.0], [1.0e-10, 10.0]),
            line([1.0e-10, 10.0], [0.0, 10.0]),
            line([0.0, 10.0], [0.0, 0.0]),
        ],
        start_mm: 0.0,
        end_mm: 20.0,
    };
    assert_eq!(btlx_free_contour(&collapsed_after_formatting), None);

    let mut excessive_depth = valid;
    let GeneralMachiningGeometry::ProfileCut { end_mm, .. } = &mut excessive_depth else {
        unreachable!()
    };
    *end_mm = 100_011.0;
    assert_eq!(btlx_free_contour(&excessive_depth), None);
}
