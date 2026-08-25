use ketchup_core::document::{DocumentStore, FeatureKind, ProfileSegment};
use ketchup_core::graph::sha256_bytes;
use ketchup_core::import::{
    DxfImportError, DxfImportOptions, ImportFormat, ImportLengthUnit, ImportUnitAuthority,
    inspect_dxf, plan_dxf_import,
};
use ketchup_core::persistence;

fn dxf(insunits: Option<i16>, entities: &str) -> Vec<u8> {
    let units = insunits
        .map(|value| format!("9\n$INSUNITS\n70\n{value}\n"))
        .unwrap_or_default();
    format!(
        "0\nSECTION\n2\nHEADER\n{units}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn dxf_with_blocks(insunits: i16, blocks: &str, entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n{insunits}\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn disconnected_lines(count: usize) -> Vec<u8> {
    let mut entities = String::new();
    for index in 0..count {
        let x = index * 3;
        entities.push_str(&format!(
            "0\nLINE\n8\n0\n10\n{x}\n20\n0\n11\n{}\n21\n0\n",
            x + 1
        ));
    }
    dxf(Some(4), &entities)
}

fn supported_fixture() -> Vec<u8> {
    dxf(
        Some(4),
        concat!(
            "0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n",
            "0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n",
            "0\nLWPOLYLINE\n8\ncut\n90\n4\n70\n1\n43\n2\n",
            "10\n0\n20\n0\n42\n0\n",
            "10\n10\n20\n0\n42\n1\n",
            "10\n10\n20\n10\n42\n0\n",
            "10\n0\n20\n10\n42\n0\n",
            "0\nELLIPSE\n8\nunsupported\n10\n0\n20\n0\n11\n5\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n"
        ),
    )
}

#[test]
fn supported_lines_arcs_polylines_bulges_layers_and_losses_are_canonical() {
    let parsed = inspect_dxf(&supported_fixture(), DxfImportOptions::new(None)).unwrap();

    assert_eq!(
        parsed.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert_eq!(parsed.units().source_unit(), ImportLengthUnit::Millimetre);
    assert_eq!(parsed.layers(), &["cut".to_owned(), "outline".to_owned()]);
    assert_eq!(parsed.profiles().len(), 2);
    let cut = &parsed.profiles()[0];
    assert_eq!(cut.layer(), "cut");
    assert!(cut.closed());
    assert_eq!(cut.segments().len(), 4);
    assert!(matches!(
        cut.segments()[1],
        ProfileSegment::CircularArc {
            clockwise: false,
            ..
        }
    ));
    let outline = &parsed.profiles()[1];
    assert_eq!(outline.layer(), "outline");
    assert!(!outline.closed());
    assert_eq!(outline.segments().len(), 2);
    assert!(matches!(outline.segments()[0], ProfileSegment::Line { .. }));
    assert!(matches!(
        outline.segments()[1],
        ProfileSegment::CircularArc {
            clockwise: false,
            ..
        }
    ));
    assert!(
        parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "dxf.entity-unsupported"
                && diagnostic.subject() == Some("ELLIPSE"))
    );
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-group-ignored"
            && diagnostic.subject() == Some("LWPOLYLINE:43")
    }));
    assert!(
        parsed
            .diagnostics()
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
}

#[test]
fn circles_become_closed_exact_two_arc_profiles_with_scaled_units() {
    let source = dxf(
        Some(5),
        "0\nCIRCLE\n8\nbores\n10\n2\n20\n3\n40\n0.5\n62\n1\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["bores".to_owned()]);
    assert_eq!(parsed.profiles().len(), 1);
    let circle = &parsed.profiles()[0];
    assert_eq!(circle.layer(), "bores");
    assert!(circle.closed());
    assert_eq!(circle.segments().len(), 2);
    assert_eq!(circle.segments()[0].start_mm(), [25.0, 30.0]);
    assert_eq!(circle.segments()[0].end_mm(), [15.0, 30.0]);
    assert_eq!(
        circle.segments()[1].start_mm(),
        circle.segments()[0].end_mm()
    );
    assert_eq!(
        circle.segments()[1].end_mm(),
        circle.segments()[0].start_mm()
    );
    for segment in circle.segments() {
        assert!(matches!(
            segment,
            ProfileSegment::CircularArc {
                center_mm: [20.0, 30.0],
                clockwise: false,
                ..
            }
        ));
    }
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-group-ignored" && diagnostic.subject() == Some("CIRCLE:62")
    }));
}

#[test]
fn invalid_circles_fail_closed_with_typed_errors() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nCIRCLE\n10\n0\n20\n0\n40\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nCIRCLE\n10\n0\n20\n0\n40\n-1\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nCIRCLE\n10\n0\n20\n0\n30\n1\n40\n1\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nCIRCLE\n10\n0\n20\n0\n39\n1\n40\n1\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nCIRCLE\n10\n999999\n20\n0\n40\n2\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        ("0\nCIRCLE\n10\n0\n20\n0\n", DxfImportError::MalformedPairs),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn circular_ellipses_become_exact_full_and_partial_arc_profiles() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nELLIPSE\n8\nfull\n10\n1\n20\n2\n11\n0\n21\n0.2\n40\n1\n62\n1\n",
            "0\nELLIPSE\n8\npartial\n10\n5\n20\n6\n11\n0.2\n21\n0\n40\n1\n41\n0\n42\n1.5707963267948966\n",
            "0\nELLIPSE\n8\nunsupported\n10\n0\n20\n0\n11\n1\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["full".to_owned(), "partial".to_owned()]);
    assert_eq!(parsed.profiles().len(), 2);
    let full = &parsed.profiles()[0];
    assert!(full.closed());
    assert!(matches!(
        full.segments(),
        [
            ProfileSegment::CircularArc {
                start_mm: [10.0, 22.0],
                end_mm: [10.0, 18.0],
                center_mm: [10.0, 20.0],
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [10.0, 18.0],
                end_mm: [10.0, 22.0],
                center_mm: [10.0, 20.0],
                clockwise: false,
            }
        ]
    ));
    let partial = &parsed.profiles()[1];
    assert!(!partial.closed());
    assert!(matches!(
        partial.segments(),
        [ProfileSegment::CircularArc {
            start_mm: [52.0, 60.0],
            end_mm: [50.0, 62.0],
            center_mm: [50.0, 60.0],
            clockwise: false,
        }]
    ));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("ELLIPSE")
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-group-ignored"
            && diagnostic.subject() == Some("ELLIPSE:62")
    }));
}

#[test]
fn circular_ellipse_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nELLIPSE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n0\n40\n1\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\narcs\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["arcs".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested circular ELLIPSE must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 18.0]);
    assert_eq!(profile.segments()[1].end_mm(), [11.0, 26.0]);
}

#[test]
fn malformed_non_planar_degenerate_and_out_of_envelope_circular_ellipses_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nELLIPSE\n11\n1\n21\n0\n40\n1\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nELLIPSE\n10\n0\n20\n0\n11\n1\n21\n0\n40\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nELLIPSE\n10\n0\n20\n0\n11\n0\n21\n0\n40\n1\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nELLIPSE\n10\n0\n20\n0\n11\n1\n21\n0\n31\n1\n40\n1\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nELLIPSE\n10\n0\n20\n0\n11\n1\n21\n0\n40\n1\n41\n2\n42\n1\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nELLIPSE\n10\n999999\n20\n0\n11\n2\n21\n0\n40\n1\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            "0\nCIRCLE\n10\n0\n20\n0\n40\n1\n0\nELLIPSE\n10\n0\n20\n0\n11\n1\n21\n0\n40\n1\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nELLIPSE\n10\n0\n20\n0\n11\n1\n21\n0\n40\n1\n42\nNaN\n",
            DxfImportError::InvalidNumber,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn linear_splines_become_exact_open_and_closed_line_profiles_without_approximating_curves() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nSPLINE\n8\nclosed-spline\n70\n25\n71\n1\n72\n5\n73\n3\n74\n0\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n1\n20\n2\n30\n0\n10\n3\n20\n2\n30\n0\n10\n1\n20\n4\n30\n0\n62\n1\n",
            "0\nSPLINE\n8\nopen-spline\n70\n24\n71\n1\n72\n5\n73\n3\n74\n0\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n5\n20\n6\n10\n7\n20\n6\n10\n7\n20\n8\n",
            "0\nSPLINE\n8\ncurved\n70\n8\n71\n2\n72\n6\n73\n3\n74\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n1\n10\n2\n20\n0\n",
            "0\nSPLINE\n8\nrational\n70\n28\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n41\n1\n10\n1\n20\n0\n41\n1\n",
            "0\nSPLINE\n8\nperiodic\n70\n26\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n",
            "0\nSPLINE\n8\nfit\n70\n24\n71\n1\n72\n4\n73\n2\n74\n2\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n11\n0\n21\n0\n11\n1\n21\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &["closed-spline".to_owned(), "open-spline".to_owned()]
    );
    let closed = &parsed.profiles()[0];
    assert!(closed.closed());
    assert!(matches!(
        closed.segments(),
        [
            ProfileSegment::Line {
                start_mm: [10.0, 20.0],
                end_mm: [30.0, 20.0]
            },
            ProfileSegment::Line {
                start_mm: [30.0, 20.0],
                end_mm: [10.0, 40.0]
            },
            ProfileSegment::Line {
                start_mm: [10.0, 40.0],
                end_mm: [10.0, 20.0]
            }
        ]
    ));
    let open = &parsed.profiles()[1];
    assert!(!open.closed());
    assert!(matches!(
        open.segments(),
        [
            ProfileSegment::Line {
                start_mm: [50.0, 60.0],
                end_mm: [70.0, 60.0]
            },
            ProfileSegment::Line {
                start_mm: [70.0, 60.0],
                end_mm: [70.0, 80.0]
            }
        ]
    ));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("SPLINE")
            && diagnostic.count() == 4
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-group-ignored" && diagnostic.subject() == Some("SPLINE:62")
    }));
}

#[test]
fn linear_spline_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nSPLINE\n8\n0\n70\n25\n71\n1\n72\n5\n73\n3\n74\n0\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n1\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\npaths\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["paths".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested linear SPLINE must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [9.0, 22.0]);
    assert_eq!(profile.segments()[2].end_mm(), [11.0, 22.0]);
}

#[test]
fn malformed_non_planar_degenerate_and_ambiguous_linear_splines_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n5\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n1\n40\n0\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0.5\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n30\n1\n10\n1\n20\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n10\n0\n20\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n999999\n20\n0\n10\n1000001\n20\n0\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            "0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nSPLINE\n70\n16\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\nNaN\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::InvalidNumber,
        ),
        (
            "0\nSPLINE\n70\n20\n71\n1\n72\n4\n73\n2\n74\n0\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n41\n1\n10\n1\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn mpolygon_boundaries_become_exact_closed_profiles_with_explicit_semantic_losses() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nMPOLYGON\n8\nmpolygon\n70\n1\n10\n0\n20\n0\n30\n0\n210\n0\n220\n0\n230\n1\n2\nSOLID\n71\n1\n91\n2\n",
            "92\n3\n72\n0\n73\n1\n93\n4\n10\n1\n20\n2\n10\n5\n20\n2\n10\n5\n20\n6\n10\n1\n20\n6\n97\n0\n",
            "92\n1\n93\n4\n72\n1\n10\n10\n20\n2\n11\n14\n21\n2\n72\n2\n10\n14\n20\n4\n40\n2\n50\n270\n51\n90\n73\n1\n72\n1\n10\n14\n20\n6\n11\n10\n21\n6\n72\n1\n10\n10\n20\n6\n11\n10\n21\n2\n97\n0\n",
            "76\n1\n73\n1\n11\n0\n21\n0\n99\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["mpolygon".to_owned()]);
    let [polyline, edge] = parsed.profiles() else {
        panic!("two MPOLYGON boundaries must become two exact profiles")
    };
    assert!(polyline.closed());
    assert_eq!(polyline.segments().len(), 4);
    assert_eq!(polyline.segments()[0].start_mm(), [10.0, 20.0]);
    assert_eq!(polyline.segments()[3].end_mm(), [10.0, 20.0]);
    assert!(edge.closed());
    assert!(matches!(
        edge.segments()[1],
        ProfileSegment::CircularArc {
            start_mm: [140.0, 20.0],
            end_mm: [140.0, 60.0],
            center_mm: [140.0, 40.0],
            clockwise: false,
        }
    ));
    for code in [
        "dxf.mpolygon-fill-dropped",
        "dxf.mpolygon-annotation-dropped",
        "dxf.mpolygon-boundary-topology-dropped",
        "dxf.mpolygon-geometry",
    ] {
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == code && diagnostic.subject() == Some("mpolygon")
        }));
    }
    assert!(!parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("MPOLYGON")
    }));
}

#[test]
fn mpolygon_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nMPOLYGON\n8\n0\n70\n1\n10\n0\n20\n0\n30\n0\n71\n0\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n1\n97\n0\n76\n1\n73\n0\n11\n0\n21\n0\n99\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nmpolygons\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["mpolygons".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested MPOLYGON must expand to one exact profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [9.0, 22.0]);
}

#[test]
fn invalid_or_ambiguous_mpolygon_metadata_and_boundaries_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nMPOLYGON\n70\n2\n10\n0\n20\n0\n71\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n99\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMPOLYGON\n70\n1\n10\n1\n20\n0\n71\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n99\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nMPOLYGON\n70\n1\n10\n0\n20\n0\n71\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n11\n1\n21\n0\n99\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMPOLYGON\n70\n1\n10\n0\n20\n0\n71\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n99\n1\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMPOLYGON\n70\n1\n10\n0\n20\n0\n71\n1\n91\n1\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n1\n330\nABCD\n99\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMPOLYGON\n70\n1\n10\n0\n20\n0\n71\n1\n91\n1\n92\n2\n72\n0\n73\n0\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n99\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMPOLYGON\n70\n1\n10\n0\n20\n0\n71\n1\n91\n2\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n99\n0\n",
            DxfImportError::MalformedPairs,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn planar_mesh_exterior_becomes_one_exact_closed_profile_with_explicit_topology_loss() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nMESH\n8\nmesh\n71\n2\n72\n0\n91\n0\n92\n4\n",
            "10\n0\n20\n0\n30\n0\n10\n4\n20\n0\n30\n0\n10\n4\n20\n3\n30\n0\n10\n0\n20\n3\n30\n0\n",
            "93\n8\n90\n3\n90\n0\n90\n1\n90\n2\n90\n3\n90\n0\n90\n2\n90\n3\n94\n0\n95\n0\n90\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["mesh".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("two coplanar MESH faces must produce one exterior profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments().len(), 4);
    assert_eq!(profile.segments()[0].start_mm(), [0.0, 0.0]);
    assert_eq!(profile.segments()[0].end_mm(), [40.0, 0.0]);
    assert_eq!(profile.segments()[2].end_mm(), [0.0, 30.0]);
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.mesh-face-topology-dropped"
            && diagnostic.subject() == Some("mesh")
            && diagnostic.count() == 2
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.mesh-boundary-geometry"
            && diagnostic.subject() == Some("mesh")
            && diagnostic.count() == 1
    }));
    assert!(!parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("MESH")
    }));
}

#[test]
fn planar_mesh_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nMESH\n8\n0\n71\n2\n72\n1\n91\n0\n92\n3\n",
            "10\n0\n20\n0\n30\n0\n10\n2\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n",
            "93\n4\n90\n3\n90\n0\n90\n1\n90\n2\n94\n0\n95\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nmeshes\n2\nouter\n10\n10\n20\n20\n41\n2\n42\n2\n50\n90\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["meshes".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested planar MESH must produce one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [6.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [6.0, 26.0]);
}

#[test]
fn invalid_or_ambiguous_mesh_metadata_and_topology_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nMESH\n71\n1\n72\n0\n91\n0\n92\n3\n10\n0\n20\n0\n30\n0\n10\n1\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n93\n4\n90\n3\n90\n0\n90\n1\n90\n2\n94\n0\n95\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMESH\n71\n2\n72\n0\n91\n1\n92\n3\n10\n0\n20\n0\n30\n0\n10\n1\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n93\n4\n90\n3\n90\n0\n90\n1\n90\n2\n94\n0\n95\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMESH\n71\n2\n72\n0\n91\n0\n92\n3\n10\n0\n20\n0\n30\n1\n10\n1\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n93\n4\n90\n3\n90\n0\n90\n1\n90\n2\n94\n0\n95\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nMESH\n71\n2\n72\n0\n91\n0\n92\n3\n10\n0\n20\n0\n30\n0\n10\n1\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n93\n5\n90\n3\n90\n0\n90\n1\n90\n2\n94\n0\n95\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nMESH\n71\n2\n72\n0\n91\n0\n92\n3\n10\n0\n20\n0\n30\n0\n10\n1\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n93\n4\n90\n3\n90\n0\n90\n1\n90\n3\n94\n0\n95\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMESH\n71\n2\n72\n0\n91\n0\n92\n4\n10\n0\n20\n0\n30\n0\n10\n1\n20\n0\n30\n0\n10\n1\n20\n1\n30\n0\n10\n0\n20\n1\n30\n0\n93\n8\n90\n3\n90\n0\n90\n1\n90\n2\n90\n3\n90\n0\n90\n3\n90\n2\n94\n0\n95\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nMESH\n71\n2\n72\n0\n91\n0\n92\n3\n10\n0\n20\n0\n30\n0\n10\n1\n20\n0\n30\n0\n10\n0\n20\n1\n30\n0\n93\n4\n90\n3\n90\n0\n90\n1\n90\n2\n94\n1\n90\n0\n90\n1\n95\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn mesh_exterior_profiles_respect_the_global_profile_budget() {
    let mut entity = String::from("0\nMESH\n71\n2\n72\n0\n91\n0\n92\n513\n");
    for index in 0..171 {
        let x = index * 10;
        entity.push_str(&format!(
            "10\n{x}\n20\n0\n30\n0\n10\n{}\n20\n0\n30\n0\n10\n{x}\n20\n1\n30\n0\n",
            x + 1
        ));
    }
    entity.push_str("93\n684\n");
    for index in 0..171 {
        let first = index * 3;
        entity.push_str(&format!(
            "90\n3\n90\n{first}\n90\n{}\n90\n{}\n",
            first + 1,
            first + 2
        ));
    }
    entity.push_str("94\n0\n95\n0\n");

    assert_eq!(
        inspect_dxf(&dxf(Some(4), &entity), DxfImportOptions::new(None)),
        Err(DxfImportError::TooManyProfiles)
    );
}

#[test]
fn legacy_polygon_mesh_exteriors_become_exact_closed_profiles_with_explicit_surface_loss() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nPOLYLINE\n8\nopen-grid\n66\n1\n70\n16\n71\n3\n72\n2\n73\n0\n74\n0\n75\n0\n",
            "0\nVERTEX\n8\nopen-grid\n70\n64\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nopen-grid\n70\n64\n10\n0\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\nopen-grid\n70\n64\n10\n2\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nopen-grid\n70\n64\n10\n2\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\nopen-grid\n70\n64\n10\n4\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nopen-grid\n70\n64\n10\n4\n20\n3\n30\n0\n",
            "0\nSEQEND\n8\nopen-grid\n",
            "0\nPOLYLINE\n8\nclosed-grid\n66\n1\n70\n17\n71\n4\n72\n2\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n11\n20\n1\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n10\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n13\n20\n1\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n14\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n13\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n14\n20\n4\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n11\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\nclosed-grid\n70\n64\n10\n10\n20\n4\n30\n0\n",
            "0\nSEQEND\n8\nclosed-grid\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &["closed-grid".to_owned(), "open-grid".to_owned()]
    );
    assert_eq!(parsed.profiles().len(), 3);
    assert!(parsed.profiles().iter().all(|profile| profile.closed()));
    let open = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "open-grid")
        .unwrap();
    assert_eq!(open.segments().len(), 6);
    assert_eq!(open.segments()[0].start_mm(), [0.0, 0.0]);
    assert_eq!(open.segments()[0].end_mm(), [20.0, 0.0]);
    assert_eq!(
        parsed
            .profiles()
            .iter()
            .filter(|profile| profile.layer() == "closed-grid")
            .count(),
        2
    );
    for (layer, faces, boundaries) in [("open-grid", 2, 1), ("closed-grid", 4, 2)] {
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "dxf.polygon-mesh-surface-topology-dropped"
                && diagnostic.subject() == Some(layer)
                && diagnostic.count() == faces
        }));
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "dxf.polygon-mesh-boundary-geometry"
                && diagnostic.subject() == Some(layer)
                && diagnostic.count() == boundaries
        }));
    }
}

#[test]
fn legacy_polygon_mesh_closed_in_n_direction_produces_two_exact_boundaries() {
    let source = dxf(
        Some(4),
        concat!(
            "0\nPOLYLINE\n8\nn-closed\n66\n1\n70\n48\n71\n2\n72\n4\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n21\n20\n1\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n23\n20\n1\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n23\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n21\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n20\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n24\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n24\n20\n4\n30\n0\n",
            "0\nVERTEX\n8\nn-closed\n70\n64\n10\n20\n20\n4\n30\n0\n",
            "0\nSEQEND\n8\nn-closed\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.profiles().len(), 2);
    assert!(
        parsed
            .profiles()
            .iter()
            .all(|profile| profile.closed() && profile.segments().len() == 4)
    );
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.polygon-mesh-surface-topology-dropped"
            && diagnostic.subject() == Some("n-closed")
            && diagnostic.count() == 4
    }));
}

#[test]
fn legacy_polygon_mesh_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n0\n20\n0\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nPOLYLINE\n8\n0\n66\n1\n70\n16\n71\n2\n72\n2\n",
            "0\nVERTEX\n8\n0\n70\n64\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\n0\n70\n64\n10\n0\n20\n1\n30\n0\n",
            "0\nVERTEX\n8\n0\n70\n64\n10\n2\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\n0\n70\n64\n10\n2\n20\n1\n30\n0\n",
            "0\nSEQEND\n8\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\ngrids\n2\nouter\n10\n10\n20\n20\n41\n2\n42\n2\n50\n90\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["grids".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested planar polygon mesh must produce one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [10.0, 20.0]);
    assert_eq!(profile.segments()[0].end_mm(), [10.0, 24.0]);
}

#[test]
fn invalid_or_ambiguous_legacy_polygon_mesh_metadata_and_geometry_fail_closed() {
    let options = DxfImportOptions::new(None);
    let vertex = |flags: i32, x: i32, y: i32, z: i32| {
        format!("0\nVERTEX\n70\n{flags}\n10\n{x}\n20\n{y}\n30\n{z}\n")
    };
    let valid_vertices = format!(
        "{}{}{}{}",
        vertex(64, 0, 0, 0),
        vertex(64, 0, 1, 0),
        vertex(64, 1, 0, 0),
        vertex(64, 1, 1, 0)
    );
    for (entity, expected) in [
        (
            format!("0\nPOLYLINE\n66\n1\n70\n16\n71\n3\n72\n2\n{valid_vertices}0\nSEQEND\n"),
            DxfImportError::MalformedPairs,
        ),
        (
            format!("0\nPOLYLINE\n66\n1\n70\n16\n71\n2\n72\n2\n75\n5\n{valid_vertices}0\nSEQEND\n"),
            DxfImportError::AmbiguousGeometry,
        ),
        (
            format!("0\nPOLYLINE\n66\n1\n70\n49\n71\n2\n72\n2\n{valid_vertices}0\nSEQEND\n"),
            DxfImportError::AmbiguousGeometry,
        ),
        (
            format!(
                "0\nPOLYLINE\n66\n1\n70\n16\n71\n2\n72\n2\n{}{}{}{}0\nSEQEND\n",
                vertex(0, 0, 0, 0),
                vertex(64, 1, 0, 0),
                vertex(64, 0, 1, 0),
                vertex(64, 1, 1, 0)
            ),
            DxfImportError::AmbiguousGeometry,
        ),
        (
            format!(
                "0\nPOLYLINE\n66\n1\n70\n16\n71\n2\n72\n2\n{}{}{}{}0\nSEQEND\n",
                vertex(64, 0, 0, 1),
                vertex(64, 1, 0, 0),
                vertex(64, 0, 1, 0),
                vertex(64, 1, 1, 0)
            ),
            DxfImportError::NonPlanarGeometry,
        ),
        (
            format!(
                "0\nPOLYLINE\n66\n1\n70\n16\n71\n2\n72\n2\n{}{}{}{}0\nSEQEND\n",
                vertex(64, 0, 0, 0),
                vertex(64, 1, 0, 0),
                vertex(64, 0, 0, 0),
                vertex(64, 1, 1, 0)
            ),
            DxfImportError::AmbiguousGeometry,
        ),
        (
            concat!(
                "0\nPOLYLINE\n66\n1\n70\n16\n71\n2\n72\n2\n",
                "0\nVERTEX\n70\n64\n10\n0\n20\n0\n30\n0\n42\n1\n",
                "0\nVERTEX\n70\n64\n10\n1\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n64\n10\n0\n20\n1\n30\n0\n",
                "0\nVERTEX\n70\n64\n10\n1\n20\n1\n30\n0\n",
                "0\nSEQEND\n"
            )
            .to_owned(),
            DxfImportError::InvalidBulge,
        ),
        (
            format!(
                "0\nPOLYLINE\n66\n1\n70\n16\n71\n2\n72\n2\n{}{}{}{}0\nSEQEND\n",
                vertex(64, 0, 0, 0),
                vertex(64, 1, 1, 0),
                vertex(64, 0, 1, 0),
                vertex(64, 1, 0, 0)
            ),
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), &entity), options), Err(expected));
    }
}

#[test]
fn legacy_polygon_mesh_exterior_profiles_respect_the_global_profile_budget() {
    let mut entities = String::new();
    for index in 0..86 {
        let x = index * 10;
        entities.push_str(&format!(
            "0\nPOLYLINE\n8\ngrids\n66\n1\n70\n17\n71\n3\n72\n2\n\
             0\nVERTEX\n8\ngrids\n70\n64\n10\n{}\n20\n1\n30\n0\n\
             0\nVERTEX\n8\ngrids\n70\n64\n10\n{x}\n20\n0\n30\n0\n\
             0\nVERTEX\n8\ngrids\n70\n64\n10\n{}\n20\n1\n30\n0\n\
             0\nVERTEX\n8\ngrids\n70\n64\n10\n{}\n20\n0\n30\n0\n\
             0\nVERTEX\n8\ngrids\n70\n64\n10\n{}.5\n20\n2\n30\n0\n\
             0\nVERTEX\n8\ngrids\n70\n64\n10\n{}.5\n20\n3\n30\n0\n\
             0\nSEQEND\n8\ngrids\n",
            x + 1,
            x + 2,
            x + 3,
            x + 1,
            x + 1,
        ));
    }

    assert_eq!(
        inspect_dxf(&dxf(Some(4), &entities), DxfImportOptions::new(None)),
        Err(DxfImportError::TooManyProfiles)
    );
}

#[test]
fn legacy_polyface_mesh_exterior_becomes_one_exact_closed_profile_with_explicit_losses() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nPOLYLINE\n8\npolyface\n66\n1\n70\n64\n71\n4\n72\n2\n",
            "0\nVERTEX\n8\npolyface\n70\n192\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\npolyface\n70\n192\n10\n4\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\npolyface\n70\n192\n10\n4\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\npolyface\n70\n192\n10\n0\n20\n3\n30\n0\n",
            "0\nVERTEX\n8\npolyface\n70\n128\n71\n1\n72\n2\n73\n-3\n",
            "0\nVERTEX\n8\npolyface\n70\n128\n71\n1\n72\n3\n73\n4\n",
            "0\nSEQEND\n8\npolyface\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["polyface".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("two coplanar polyface faces must produce one exterior profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments().len(), 4);
    assert_eq!(profile.segments()[0].start_mm(), [0.0, 0.0]);
    assert_eq!(profile.segments()[0].end_mm(), [40.0, 0.0]);
    assert_eq!(profile.segments()[2].end_mm(), [0.0, 30.0]);
    for (code, count) in [
        ("dxf.polyface-face-topology-dropped", 2),
        ("dxf.polyface-boundary-geometry", 1),
        ("dxf.polyface-invisible-edge-dropped", 1),
    ] {
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == code
                && diagnostic.subject() == Some("polyface")
                && diagnostic.count() == count
        }));
    }
}

#[test]
fn legacy_polyface_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nPOLYLINE\n8\n0\n66\n1\n70\n64\n71\n3\n72\n1\n",
            "0\nVERTEX\n8\n0\n70\n192\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\n0\n70\n192\n10\n2\n20\n0\n30\n0\n",
            "0\nVERTEX\n8\n0\n70\n192\n10\n0\n20\n1\n30\n0\n",
            "0\nVERTEX\n8\n0\n70\n128\n71\n1\n72\n2\n73\n3\n",
            "0\nSEQEND\n8\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\npolyfaces\n2\nouter\n10\n10\n20\n20\n41\n2\n42\n2\n50\n90\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["polyfaces".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested planar polyface must produce one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [6.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [6.0, 26.0]);
}

#[test]
fn invalid_or_ambiguous_legacy_polyface_metadata_and_topology_fail_closed() {
    let options = DxfImportOptions::new(None);
    let coordinate = |x: i32, y: i32| format!("0\nVERTEX\n70\n192\n10\n{x}\n20\n{y}\n30\n0\n");
    for (entity, expected) in [
        (
            concat!(
                "0\nPOLYLINE\n66\n1\n70\n64\n71\n4\n72\n1\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n1\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n1\n30\n0\n",
                "0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n3\n",
                "0\nSEQEND\n"
            )
            .to_owned(),
            DxfImportError::MalformedPairs,
        ),
        (
            concat!(
                "0\nPOLYLINE\n66\n1\n70\n64\n71\n3\n72\n1\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n1\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n1\n30\n0\n",
                "0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n4\n",
                "0\nSEQEND\n"
            )
            .to_owned(),
            DxfImportError::AmbiguousGeometry,
        ),
        (
            concat!(
                "0\nPOLYLINE\n66\n1\n70\n64\n71\n3\n72\n1\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n0\n30\n1\n",
                "0\nVERTEX\n70\n192\n10\n1\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n1\n30\n0\n",
                "0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n3\n",
                "0\nSEQEND\n"
            )
            .to_owned(),
            DxfImportError::NonPlanarGeometry,
        ),
        (
            concat!(
                "0\nPOLYLINE\n66\n1\n70\n64\n71\n3\n72\n1\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n1\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n192\n10\n0\n20\n0\n30\n0\n",
                "0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n3\n",
                "0\nSEQEND\n"
            )
            .to_owned(),
            DxfImportError::AmbiguousGeometry,
        ),
        (
            format!(
                "0\nPOLYLINE\n66\n1\n70\n64\n71\n5\n72\n3\n{}{}{}{}{}0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n3\n0\nVERTEX\n70\n128\n71\n2\n72\n1\n73\n4\n0\nVERTEX\n70\n128\n71\n1\n72\n2\n73\n5\n0\nSEQEND\n",
                coordinate(0, 0),
                coordinate(1, 0),
                coordinate(0, 1),
                coordinate(0, -1),
                coordinate(1, 1),
            ),
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), &entity), options), Err(expected));
    }
}

#[test]
fn legacy_polyface_exterior_profiles_respect_the_global_profile_budget() {
    let mut entity = String::from("0\nPOLYLINE\n66\n1\n70\n64\n71\n513\n72\n171\n");
    for index in 0..171 {
        let x = index * 10;
        entity.push_str(&format!(
            "0\nVERTEX\n70\n192\n10\n{x}\n20\n0\n30\n0\n0\nVERTEX\n70\n192\n10\n{}\n20\n0\n30\n0\n0\nVERTEX\n70\n192\n10\n{x}\n20\n1\n30\n0\n",
            x + 1
        ));
    }
    for index in 0..171 {
        let first = index * 3 + 1;
        entity.push_str(&format!(
            "0\nVERTEX\n70\n128\n71\n{first}\n72\n{}\n73\n{}\n",
            first + 1,
            first + 2
        ));
    }
    entity.push_str("0\nSEQEND\n");

    assert_eq!(
        inspect_dxf(&dxf(Some(4), &entity), DxfImportOptions::new(None)),
        Err(DxfImportError::TooManyProfiles)
    );
}

#[test]
fn single_polyline_boundary_hatches_become_exact_closed_profiles_without_fill_approximation() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nHATCH\n8\nhatch-lines\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n4\n10\n1\n20\n2\n10\n3\n20\n2\n10\n3\n20\n4\n10\n1\n20\n4\n97\n0\n75\n0\n76\n1\n98\n0\n",
            "0\nHATCH\n8\nhatch-bulge\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n1\n91\n1\n92\n18\n72\n1\n73\n1\n93\n3\n10\n5\n20\n6\n42\n1\n10\n7\n20\n6\n42\n0\n10\n5\n20\n8\n42\n0\n97\n0\n75\n0\n76\n1\n98\n0\n",
            "0\nHATCH\n8\npatterned-polyline\n2\nANSI31\n70\n0\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n9\n20\n2\n10\n11\n20\n2\n10\n9\n20\n4\n97\n0\n75\n0\n76\n1\n52\n45\n41\n1\n77\n0\n78\n1\n53\n45\n43\n0\n44\n0\n45\n0\n46\n1\n79\n0\n98\n0\n",
            "0\nHATCH\n8\npatterned-edge\n2\nANSI31\n70\n0\n71\n1\n91\n1\n92\n1\n93\n4\n72\n1\n10\n13\n20\n0\n11\n15\n21\n0\n72\n2\n10\n15\n20\n1\n40\n1\n50\n270\n51\n90\n73\n1\n72\n1\n10\n15\n20\n2\n11\n13\n21\n2\n72\n1\n10\n13\n20\n2\n11\n13\n21\n0\n97\n0\n75\n0\n76\n1\n52\n45\n41\n1\n77\n0\n78\n1\n53\n45\n43\n0\n44\n0\n45\n0\n46\n1\n79\n0\n98\n0\n",
            "0\nHATCH\n8\nislands\n70\n1\n71\n0\n91\n2\n92\n3\n72\n0\n73\n1\n93\n4\n10\n20\n20\n0\n10\n26\n20\n0\n10\n26\n20\n6\n10\n20\n20\n6\n97\n0\n92\n2\n72\n0\n73\n1\n93\n4\n10\n22\n20\n2\n10\n24\n20\n2\n10\n24\n20\n4\n10\n22\n20\n4\n97\n0\n75\n0\n76\n1\n98\n0\n",
            "0\nHATCH\n8\nelliptic-edge-path\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n1\n21\n0\n40\n0.5\n50\n0\n51\n360\n73\n1\n97\n0\n",
            "0\nHATCH\n8\nderived\n70\n1\n71\n0\n91\n1\n92\n7\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &[
            "hatch-bulge".to_owned(),
            "hatch-lines".to_owned(),
            "islands".to_owned(),
            "patterned-edge".to_owned(),
            "patterned-polyline".to_owned(),
        ]
    );
    let bulged = &parsed.profiles()[0];
    assert!(bulged.closed());
    assert!(matches!(
        bulged.segments(),
        [
            ProfileSegment::CircularArc {
                start_mm: [50.0, 60.0],
                end_mm: [70.0, 60.0],
                center_mm: [60.0, 60.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [70.0, 60.0],
                end_mm: [50.0, 80.0],
            },
            ProfileSegment::Line {
                start_mm: [50.0, 80.0],
                end_mm: [50.0, 60.0],
            }
        ]
    ));
    let lines = &parsed.profiles()[1];
    assert!(lines.closed());
    assert_eq!(lines.segments().len(), 4);
    assert_eq!(lines.segments()[0].start_mm(), [10.0, 20.0]);
    assert_eq!(lines.segments()[3].end_mm(), [10.0, 20.0]);
    let islands = &parsed.profiles()[2..4];
    assert!(islands.iter().all(|profile| profile.closed()));
    assert_eq!(islands[0].segments()[0].start_mm(), [200.0, 0.0]);
    assert_eq!(islands[1].segments()[0].start_mm(), [220.0, 20.0]);
    let patterned_edge = &parsed.profiles()[4];
    assert!(patterned_edge.closed());
    assert!(matches!(
        patterned_edge.segments(),
        [
            ProfileSegment::Line {
                start_mm: [130.0, 0.0],
                end_mm: [150.0, 0.0],
            },
            ProfileSegment::CircularArc {
                start_mm: [150.0, 0.0],
                end_mm: [150.0, 20.0],
                center_mm: [150.0, 10.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [150.0, 20.0],
                end_mm: [130.0, 20.0],
            },
            ProfileSegment::Line {
                start_mm: [130.0, 20.0],
                end_mm: [130.0, 0.0],
            }
        ]
    ));
    let patterned_polyline = &parsed.profiles()[5];
    assert!(patterned_polyline.closed());
    assert_eq!(patterned_polyline.segments()[0].start_mm(), [90.0, 20.0]);
    assert_eq!(patterned_polyline.segments()[2].end_mm(), [90.0, 20.0]);
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("HATCH")
            && diagnostic.count() == 2
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-boundary-topology-dropped"
            && diagnostic.subject() == Some("islands")
            && diagnostic.count() == 2
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-fill-dropped" && diagnostic.subject() == Some("hatch-lines")
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-associativity-dropped"
            && diagnostic.subject() == Some("hatch-bulge")
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-fill-dropped"
            && diagnostic.subject() == Some("patterned-polyline")
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-associativity-dropped"
            && diagnostic.subject() == Some("patterned-edge")
    }));
}

#[test]
fn line_arc_edge_path_hatches_become_exact_closed_profiles_without_curve_approximation() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nHATCH\n8\nedge-loop\n2\nSOLID\n70\n1\n71\n1\n91\n1\n92\n1\n93\n4\n72\n1\n10\n0\n20\n0\n11\n2\n21\n0\n72\n2\n10\n2\n20\n1\n40\n1\n50\n270\n51\n90\n73\n1\n72\n1\n10\n2\n20\n2\n11\n0\n21\n2\n72\n1\n10\n0\n20\n2\n11\n0\n21\n0\n97\n0\n",
            "0\nHATCH\n8\nfull-circle\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n17\n93\n1\n72\n2\n10\n5\n20\n5\n40\n2\n50\n0\n51\n360\n73\n0\n97\n0\n",
            "0\nHATCH\n8\nspline-edge\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n3\n73\n0\n74\n0\n95\n8\n96\n4\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n10\n0\n20\n1\n97\n0\n97\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &["edge-loop".to_owned(), "full-circle".to_owned()]
    );
    let edge_loop = &parsed.profiles()[0];
    assert!(edge_loop.closed());
    assert!(matches!(
        edge_loop.segments(),
        [
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [20.0, 0.0],
            },
            ProfileSegment::CircularArc {
                start_mm: [20.0, 0.0],
                end_mm: [20.0, 20.0],
                center_mm: [20.0, 10.0],
                clockwise: false,
            },
            ProfileSegment::Line {
                start_mm: [20.0, 20.0],
                end_mm: [0.0, 20.0],
            },
            ProfileSegment::Line {
                start_mm: [0.0, 20.0],
                end_mm: [0.0, 0.0],
            }
        ]
    ));
    let full_circle = &parsed.profiles()[1];
    assert!(full_circle.closed());
    assert_eq!(full_circle.segments().len(), 2);
    assert!(full_circle.segments().iter().all(|segment| matches!(
        segment,
        ProfileSegment::CircularArc {
            clockwise: true,
            ..
        }
    )));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("HATCH")
            && diagnostic.count() == 1
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-associativity-dropped"
            && diagnostic.subject() == Some("edge-loop")
    }));
}

#[test]
fn circular_elliptic_edge_path_hatches_preserve_rotated_direction_and_full_circles() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nHATCH\n8\nccw\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n4\n72\n1\n10\n0\n20\n0\n11\n2\n21\n0\n72\n3\n10\n2\n20\n1\n11\n0\n21\n1\n40\n1\n50\n180\n51\n360\n73\n1\n72\n1\n10\n2\n20\n2\n11\n0\n21\n2\n72\n1\n10\n0\n20\n2\n11\n0\n21\n0\n97\n0\n",
            "0\nHATCH\n8\ncw\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n4\n72\n1\n10\n3\n20\n0\n11\n5\n21\n0\n72\n3\n10\n5\n20\n1\n11\n0\n21\n1\n40\n1\n50\n180\n51\n0\n73\n0\n72\n1\n10\n5\n20\n2\n11\n3\n21\n2\n72\n1\n10\n3\n20\n2\n11\n3\n21\n0\n97\n0\n",
            "0\nHATCH\n8\nfull\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n17\n93\n1\n72\n3\n10\n8\n20\n1\n11\n1\n21\n1\n40\n1\n50\n45\n51\n405\n73\n0\n97\n0\n",
            "0\nHATCH\n8\ntrue-ellipse\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n2\n21\n0\n40\n0.5\n50\n0\n51\n360\n73\n1\n97\n0\n",
            "0\nHATCH\n8\nspline\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n3\n73\n0\n74\n0\n95\n8\n96\n4\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n10\n0\n20\n1\n97\n0\n97\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &["ccw".to_owned(), "cw".to_owned(), "full".to_owned()]
    );
    let ccw = &parsed.profiles()[0];
    assert!(ccw.closed());
    assert!(matches!(
        ccw.segments()[1],
        ProfileSegment::CircularArc {
            start_mm: [20.0, 0.0],
            end_mm: [20.0, 20.0],
            center_mm: [20.0, 10.0],
            clockwise: false,
        }
    ));
    let cw = &parsed.profiles()[1];
    assert!(cw.closed());
    assert!(matches!(
        cw.segments()[1],
        ProfileSegment::CircularArc {
            start_mm: [50.0, 0.0],
            end_mm: [50.0, 20.0],
            center_mm: [50.0, 10.0],
            clockwise: true,
        }
    ));
    let full = &parsed.profiles()[2];
    assert!(full.closed());
    assert_eq!(full.segments().len(), 2);
    assert!(full.segments().iter().all(|segment| matches!(
        segment,
        ProfileSegment::CircularArc {
            center_mm: [80.0, 10.0],
            clockwise: true,
            ..
        }
    )));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("HATCH")
            && diagnostic.count() == 2
    }));
    assert_eq!(
        parsed
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == "dxf.hatch-fill-dropped")
            .count(),
        3
    );
}

#[test]
fn linear_spline_edge_path_hatches_become_exact_lines_without_curve_approximation() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nHATCH\n8\nlinear-spline\n2\nSOLID\n70\n1\n71\n1\n91\n1\n92\n1\n93\n3\n72\n4\n94\n1\n73\n0\n74\n0\n95\n5\n96\n3\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n0\n20\n0\n10\n2\n20\n0\n10\n2\n20\n2\n97\n0\n72\n1\n10\n2\n20\n2\n11\n0\n21\n2\n72\n1\n10\n0\n20\n2\n11\n0\n21\n0\n97\n0\n",
            "0\nHATCH\n8\nrational-spline\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n1\n73\n1\n74\n0\n95\n4\n96\n2\n40\n0\n40\n0\n40\n1\n40\n1\n10\n5\n20\n0\n42\n1\n10\n6\n20\n0\n42\n1\n97\n0\n97\n0\n",
            "0\nHATCH\n8\nperiodic-spline\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n1\n73\n0\n74\n1\n95\n4\n96\n2\n40\n0\n40\n0\n40\n1\n40\n1\n10\n7\n20\n0\n10\n8\n20\n0\n97\n0\n97\n0\n",
            "0\nHATCH\n8\nfit-spline\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n1\n73\n0\n74\n0\n95\n4\n96\n2\n40\n0\n40\n0\n40\n1\n40\n1\n10\n9\n20\n0\n10\n10\n20\n0\n97\n1\n11\n9.5\n21\n0\n97\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["linear-spline".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("linear spline-edge HATCH must produce one exact profile")
    };
    assert!(profile.closed());
    assert!(matches!(
        profile.segments(),
        [
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [20.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [20.0, 0.0],
                end_mm: [20.0, 20.0],
            },
            ProfileSegment::Line {
                start_mm: [20.0, 20.0],
                end_mm: [0.0, 20.0],
            },
            ProfileSegment::Line {
                start_mm: [0.0, 20.0],
                end_mm: [0.0, 0.0],
            }
        ]
    ));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("HATCH")
            && diagnostic.count() == 3
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-associativity-dropped"
            && diagnostic.subject() == Some("linear-spline")
    }));
}

#[test]
fn polyline_boundary_hatch_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nHATCH\n8\n0\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n2\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n1\n97\n0\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0.5\n20\n0.2\n10\n1\n20\n0.2\n10\n0.5\n20\n0.5\n97\n0\n75\n0\n76\n1\n98\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nhatches\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["hatches".to_owned()]);
    let [outer, inner] = parsed.profiles() else {
        panic!("nested multi-boundary HATCH must expand to two profiles")
    };
    assert!(outer.closed());
    assert_eq!(outer.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(outer.segments()[0].end_mm(), [11.0, 26.0]);
    assert!(inner.closed());
    assert_eq!(inner.segments()[0].start_mm(), [10.6, 23.0]);
    assert_eq!(inner.segments()[0].end_mm(), [10.6, 24.0]);
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-boundary-topology-dropped"
            && diagnostic.subject() == Some("0")
            && diagnostic.count() == 2
    }));
    assert_eq!(outer.segments()[1].end_mm(), [9.0, 22.0]);
    assert_eq!(outer.segments()[2].end_mm(), [11.0, 22.0]);
}

#[test]
fn circular_elliptic_edge_hatch_inside_nested_insert_preserves_layer_and_exact_arc_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nHATCH\n8\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n4\n72\n1\n10\n0\n20\n0\n11\n2\n21\n0\n72\n3\n10\n2\n20\n1\n11\n1\n21\n0\n40\n1\n50\n270\n51\n90\n73\n1\n72\n1\n10\n2\n20\n2\n11\n0\n21\n2\n72\n1\n10\n0\n20\n2\n11\n0\n21\n0\n97\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nhatch-edges\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["hatch-edges".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested circular elliptic-edge HATCH must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert!(matches!(
        profile.segments()[1],
        ProfileSegment::CircularArc {
            start_mm: [11.0, 26.0],
            end_mm: [7.0, 26.0],
            center_mm: [9.0, 26.0],
            clockwise: false,
        }
    ));
}

#[test]
fn linear_spline_edge_hatch_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nHATCH\n8\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n3\n72\n4\n94\n1\n73\n0\n74\n0\n95\n5\n96\n3\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n0\n20\n0\n10\n2\n20\n0\n10\n2\n20\n2\n97\n0\n72\n1\n10\n2\n20\n2\n11\n0\n21\n2\n72\n1\n10\n0\n20\n2\n11\n0\n21\n0\n97\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nhatch-splines\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["hatch-splines".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested linear spline-edge HATCH must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [7.0, 26.0]);
    assert_eq!(profile.segments()[2].end_mm(), [7.0, 22.0]);
    assert_eq!(profile.segments()[3].end_mm(), [11.0, 22.0]);
}

#[test]
fn malformed_non_planar_open_degenerate_and_ambiguous_hatches_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nHATCH\n30\n1\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n0\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n2\n10\n0\n20\n0\n10\n1\n20\n0\n97\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n4\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n999999\n20\n0\n10\n1000001\n20\n0\n10\n999999\n20\n1\n97\n0\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n42\n1\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n",
            DxfImportError::InvalidBulge,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\nNaN\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n",
            DxfImportError::InvalidNumber,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n3\n72\n1\n10\n0\n20\n0\n11\n1\n21\n0\n72\n1\n10\n1\n20\n0\n11\n0\n21\n0\n72\n1\n10\n0\n20\n0\n11\n1\n21\n0\n97\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n2\n10\n0\n20\n0\n40\n0\n50\n0\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n2\n10\n999999\n20\n0\n40\n2\n50\n0\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n0\n21\n0\n40\n1\n50\n0\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n1\n21\n0\n40\n0\n50\n0\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n1\n21\n0\n40\n1\n50\n0\n51\n360\n73\n2\n97\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n999999\n20\n0\n11\n2\n21\n0\n40\n1\n50\n0\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n1\n21\n0\n31\n1\n40\n1\n50\n0\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n3\n10\n0\n20\n0\n11\n1\n21\n0\n40\n1\n50\nNaN\n51\n360\n73\n1\n97\n0\n",
            DxfImportError::InvalidNumber,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n1\n73\n0\n74\n0\n95\n4\n96\n2\n40\n0\n40\n0\n40\n1\n10\n0\n20\n0\n10\n1\n20\n0\n97\n0\n97\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n1\n73\n0\n74\n0\n95\n4\n96\n2\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n30\n1\n10\n1\n20\n0\n97\n0\n97\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n1\n93\n1\n72\n4\n94\n1\n73\n0\n74\n0\n95\n4\n96\n2\n40\n0\n40\n0\n40\n1\n40\n1\n10\n0\n20\n0\n10\n0\n20\n0\n97\n0\n97\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            concat!(
                "0\nLWPOLYLINE\n90\n3\n70\n1\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n",
                "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n1\n20\n0\n10\n0\n20\n1\n97\n0\n"
            ),
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn multi_boundary_hatches_fail_closed_on_invalid_envelopes_duplicates_and_unsupported_loops() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n2\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n2\n97\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n2\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n2\n97\n0\n92\n2\n72\n0\n73\n0\n93\n3\n10\n0.5\n20\n0.5\n10\n1\n20\n0.5\n10\n0.5\n20\n1\n97\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n2\n97\n1\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n171\n",
            DxfImportError::TooManyProfiles,
        ),
        (
            "0\nHATCH\n70\n1\n71\n0\n91\n2\n92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n2\n97\n0\n92\n2\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n2\n20\n0\n10\n0\n20\n2\n97\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }

    let source = dxf(
        Some(4),
        concat!(
            "0\nLINE\n8\nbase\n10\n10\n20\n0\n11\n12\n21\n0\n",
            "0\nHATCH\n8\npartial-must-drop\n70\n1\n71\n0\n91\n2\n",
            "92\n3\n72\n0\n73\n1\n93\n3\n10\n0\n20\n0\n10\n4\n20\n0\n10\n0\n20\n4\n97\n0\n",
            "92\n6\n72\n0\n73\n1\n93\n3\n10\n1\n20\n1\n10\n2\n20\n1\n10\n1\n20\n2\n97\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, options).unwrap();
    assert_eq!(parsed.profiles().len(), 1);
    assert_eq!(parsed.profiles()[0].layer(), "base");
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("HATCH")
            && diagnostic.count() == 1
    }));
}

#[test]
fn legacy_polylines_become_scaled_open_and_closed_line_bulge_profiles() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nPOLYLINE\n8\nlegacy-closed\n66\n1\n70\n1\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n10\n1\n20\n2\n42\n1\n",
            "0\nVERTEX\n10\n3\n20\n2\n",
            "0\nVERTEX\n10\n3\n20\n4\n",
            "0\nSEQEND\n",
            "0\nPOLYLINE\n8\nlegacy-open\n66\n1\n70\n0\n",
            "0\nVERTEX\n10\n5\n20\n6\n",
            "0\nVERTEX\n10\n7\n20\n8\n",
            "0\nSEQEND\n8\nlegacy-open\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(
        parsed.layers(),
        &["legacy-closed".to_owned(), "legacy-open".to_owned()]
    );
    let closed = &parsed.profiles()[0];
    assert!(closed.closed());
    assert_eq!(closed.segments().len(), 3);
    assert!(matches!(
        closed.segments()[0],
        ProfileSegment::CircularArc {
            start_mm: [10.0, 20.0],
            end_mm: [30.0, 20.0],
            center_mm: [20.0, 20.0],
            clockwise: false,
        }
    ));
    assert_eq!(closed.segments()[2].end_mm(), [10.0, 20.0]);

    let open = &parsed.profiles()[1];
    assert!(!open.closed());
    assert!(matches!(
        open.segments(),
        [ProfileSegment::Line {
            start_mm: [50.0, 60.0],
            end_mm: [70.0, 80.0],
        }]
    ));
}

#[test]
fn malformed_or_non_planar_legacy_polyline_sequences_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entities, expected) in [
        (
            "0\nPOLYLINE\n66\n1\n70\n8\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nPOLYLINE\n66\n1\n0\nVERTEX\n10\n0\n20\n0\n30\n1\n0\nVERTEX\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nPOLYLINE\n8\na\n66\n1\n0\nVERTEX\n8\nb\n10\n0\n20\n0\n0\nVERTEX\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nPOLYLINE\n66\n1\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n1\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nPOLYLINE\n66\n1\n0\nVERTEX\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n0\nSEQEND\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nPOLYLINE\n66\n0\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::MalformedPairs,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entities), options), Err(expected));
    }
}

#[test]
fn planar_3d_polylines_become_scaled_open_and_closed_line_profiles() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nPOLYLINE\n8\nclosed-3d\n66\n1\n70\n9\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n70\n32\n10\n1\n20\n2\n30\n0\n",
            "0\nVERTEX\n70\n32\n10\n3\n20\n2\n30\n0\n",
            "0\nVERTEX\n70\n32\n10\n1\n20\n4\n30\n0\n",
            "0\nSEQEND\n8\nclosed-3d\n",
            "0\nPOLYLINE\n8\nopen-3d\n66\n1\n70\n8\n",
            "0\nVERTEX\n70\n32\n10\n5\n20\n6\n30\n0\n",
            "0\nVERTEX\n70\n32\n10\n7\n20\n8\n30\n0\n",
            "0\nSEQEND\n8\nopen-3d\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &["closed-3d".to_owned(), "open-3d".to_owned()]
    );
    let closed = &parsed.profiles()[0];
    assert!(closed.closed());
    assert!(matches!(
        closed.segments(),
        [
            ProfileSegment::Line {
                start_mm: [10.0, 20.0],
                end_mm: [30.0, 20.0]
            },
            ProfileSegment::Line {
                start_mm: [30.0, 20.0],
                end_mm: [10.0, 40.0]
            },
            ProfileSegment::Line {
                start_mm: [10.0, 40.0],
                end_mm: [10.0, 20.0]
            }
        ]
    ));
    let open = &parsed.profiles()[1];
    assert!(!open.closed());
    assert!(matches!(
        open.segments(),
        [ProfileSegment::Line {
            start_mm: [50.0, 60.0],
            end_mm: [70.0, 80.0]
        }]
    ));
}

#[test]
fn planar_3d_polyline_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nPOLYLINE\n8\n0\n66\n1\n70\n9\n",
            "0\nVERTEX\n70\n32\n10\n0\n20\n0\n30\n0\n",
            "0\nVERTEX\n70\n32\n10\n2\n20\n0\n30\n0\n",
            "0\nVERTEX\n70\n32\n10\n0\n20\n1\n30\n0\n",
            "0\nSEQEND\n8\n0\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\npaths\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["paths".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested planar 3D POLYLINE must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments().len(), 3);
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [9.0, 22.0]);
    assert_eq!(profile.segments()[2].end_mm(), [11.0, 22.0]);
}

#[test]
fn non_planar_bulged_wrong_flag_mesh_and_polyface_3d_polylines_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entities, expected) in [
        (
            "0\nPOLYLINE\n66\n1\n70\n8\n0\nVERTEX\n70\n32\n10\n0\n20\n0\n30\n1\n0\nVERTEX\n70\n32\n10\n1\n20\n0\n30\n0\n0\nSEQEND\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nPOLYLINE\n66\n1\n70\n8\n0\nVERTEX\n70\n32\n10\n0\n20\n0\n42\n1\n0\nVERTEX\n70\n32\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::InvalidBulge,
        ),
        (
            "0\nPOLYLINE\n66\n1\n70\n8\n0\nVERTEX\n70\n0\n10\n0\n20\n0\n0\nVERTEX\n70\n32\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nPOLYLINE\n66\n1\n70\n24\n0\nVERTEX\n70\n64\n10\n0\n20\n0\n0\nVERTEX\n70\n64\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nPOLYLINE\n66\n1\n70\n64\n0\nVERTEX\n70\n64\n10\n0\n20\n0\n0\nVERTEX\n70\n64\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nPOLYLINE\n66\n1\n70\n4\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n1\n20\n0\n0\nSEQEND\n",
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entities), options), Err(expected));
    }
}

#[test]
fn solid_and_trace_become_scaled_closed_profiles_in_native_boundary_order() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nSOLID\n8\nfill\n10\n1\n20\n2\n11\n3\n21\n2\n12\n1\n22\n4\n13\n3\n23\n4\n62\n1\n",
            "0\nTRACE\n8\ntrace\n10\n5\n20\n6\n11\n7\n21\n6\n12\n5\n22\n8\n13\n5\n23\n8\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["fill".to_owned(), "trace".to_owned()]);
    assert_eq!(parsed.profiles().len(), 2);
    let solid = &parsed.profiles()[0];
    assert!(solid.closed());
    assert_eq!(solid.segments().len(), 4);
    assert_eq!(solid.segments()[0].start_mm(), [10.0, 20.0]);
    assert_eq!(solid.segments()[0].end_mm(), [30.0, 20.0]);
    assert_eq!(solid.segments()[1].end_mm(), [30.0, 40.0]);
    assert_eq!(solid.segments()[2].end_mm(), [10.0, 40.0]);
    assert_eq!(solid.segments()[3].end_mm(), [10.0, 20.0]);
    let trace = &parsed.profiles()[1];
    assert!(trace.closed());
    assert_eq!(trace.segments().len(), 3);
    assert_eq!(trace.segments()[0].start_mm(), [50.0, 60.0]);
    assert_eq!(trace.segments()[0].end_mm(), [70.0, 60.0]);
    assert_eq!(trace.segments()[1].end_mm(), [50.0, 80.0]);
    assert_eq!(trace.segments()[2].end_mm(), [50.0, 60.0]);
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-group-ignored" && diagnostic.subject() == Some("SOLID:62")
    }));
    assert!(!parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && matches!(diagnostic.subject(), Some("SOLID" | "TRACE"))
    }));
}

#[test]
fn solid_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nSOLID\n8\n0\n10\n0\n20\n0\n11\n2\n21\n0\n12\n0\n22\n1\n13\n2\n23\n1\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nparts\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["parts".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested SOLID must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments().len(), 4);
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [9.0, 26.0]);
    assert_eq!(profile.segments()[3].end_mm(), [11.0, 22.0]);
}

#[test]
fn malformed_non_planar_degenerate_duplicate_and_crossed_traces_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nSOLID\n10\n0\n20\n0\n11\n1\n21\n0\n12\n0\n22\n1\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nTRACE\n10\n0\n20\n0\n11\n1\n21\n0\n12\n0\n22\n1\n13\n0\n23\n1\n33\n1\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nSOLID\n10\n0\n20\n0\n11\n1\n21\n0\n12\n2\n22\n0\n13\n2\n23\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nTRACE\n10\n0\n20\n0\n11\n0\n21\n0\n12\n0\n22\n1\n13\n1\n23\n1\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\nSOLID\n10\n0\n20\n0\n11\n2\n21\n2\n12\n2\n22\n0\n13\n0\n23\n2\n",
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn straight_leader_becomes_scaled_open_profile_with_explicit_semantic_losses() {
    let source = dxf(
        Some(5),
        concat!(
            "0\nLEADER\n8\nleaders\n3\nSTANDARD\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n3\n",
            "10\n1\n20\n2\n30\n0\n10\n3\n20\n2\n30\n0\n10\n3\n20\n4\n30\n0\n340\nABCD\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["leaders".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("straight LEADER must become one profile")
    };
    assert!(!profile.closed());
    assert_eq!(profile.segments().len(), 2);
    assert_eq!(profile.segments()[0].start_mm(), [10.0, 20.0]);
    assert_eq!(profile.segments()[0].end_mm(), [30.0, 20.0]);
    assert_eq!(profile.segments()[1].end_mm(), [30.0, 40.0]);
    for code in [
        "dxf.leader-arrowhead-dropped",
        "dxf.leader-semantics-dropped",
        "dxf.leader-geometry",
    ] {
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code() == code && diagnostic.count() == 1 })
        );
    }
    assert!(!parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("LEADER")
    }));
}

#[test]
fn straight_leader_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\nLEADER\n8\n0\n71\n0\n72\n0\n73\n3\n74\n0\n75\n0\n76\n3\n",
            "10\n0\n20\n0\n10\n2\n20\n0\n10\n2\n20\n1\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nleaders\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["leaders".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested straight LEADER must expand to one profile")
    };
    assert!(!profile.closed());
    assert_eq!(profile.segments().len(), 2);
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [9.0, 26.0]);
}

#[test]
fn spline_and_hookline_leaders_are_unsupported_without_approximation() {
    let source = dxf(
        Some(4),
        concat!(
            "0\nLINE\n8\nbase\n10\n0\n20\n0\n11\n1\n21\n0\n",
            "0\nLEADER\n8\nleaders\n71\n1\n72\n1\n73\n0\n74\n0\n75\n0\n76\n2\n10\n2\n20\n0\n10\n3\n20\n1\n",
            "0\nLEADER\n8\nleaders\n71\n1\n72\n0\n73\n0\n74\n1\n75\n1\n76\n2\n10\n4\n20\n0\n10\n5\n20\n1\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.profiles().len(), 1);
    assert_eq!(parsed.profiles()[0].layer(), "base");
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("LEADER")
            && diagnostic.count() == 2
    }));
}

#[test]
fn malformed_non_planar_degenerate_and_out_of_range_leaders_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\nLEADER\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n3\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nLEADER\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n10\n0\n10\n1\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nLEADER\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n10\n0\n20\n0\n30\n1\n10\n1\n20\n0\n30\n0\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\nLEADER\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n10\n0\n20\n0\n10\n0\n20\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\nLEADER\n71\n2\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n10\n0\n20\n0\n10\n1\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\nLEADER\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n2\n10\n0\n20\n0\n10\n1000001\n20\n0\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            "0\nLEADER\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n1026\n",
            DxfImportError::TooManySegments,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn planar_3dface_becomes_scaled_closed_profiles_in_native_boundary_order() {
    let source = dxf(
        Some(5),
        concat!(
            "0\n3DFACE\n8\nfaces\n10\n1\n20\n2\n11\n3\n21\n2\n12\n3\n22\n4\n13\n1\n23\n4\n62\n1\n",
            "0\n3DFACE\n8\ntriangles\n10\n5\n20\n6\n11\n7\n21\n6\n12\n5\n22\n8\n13\n5\n23\n8\n70\n0\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(
        parsed.layers(),
        &["faces".to_owned(), "triangles".to_owned()]
    );
    let quad = &parsed.profiles()[0];
    assert!(quad.closed());
    assert_eq!(quad.segments().len(), 4);
    assert_eq!(quad.segments()[0].start_mm(), [10.0, 20.0]);
    assert_eq!(quad.segments()[0].end_mm(), [30.0, 20.0]);
    assert_eq!(quad.segments()[1].end_mm(), [30.0, 40.0]);
    assert_eq!(quad.segments()[2].end_mm(), [10.0, 40.0]);
    assert_eq!(quad.segments()[3].end_mm(), [10.0, 20.0]);
    let triangle = &parsed.profiles()[1];
    assert!(triangle.closed());
    assert_eq!(triangle.segments().len(), 3);
    assert_eq!(triangle.segments()[0].start_mm(), [50.0, 60.0]);
    assert_eq!(triangle.segments()[0].end_mm(), [70.0, 60.0]);
    assert_eq!(triangle.segments()[1].end_mm(), [50.0, 80.0]);
    assert_eq!(triangle.segments()[2].end_mm(), [50.0, 60.0]);
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-group-ignored" && diagnostic.subject() == Some("3DFACE:62")
    }));
    assert!(!parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("3DFACE")
    }));
}

#[test]
fn planar_3dface_inside_nested_insert_preserves_layer_and_exact_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\nleaf\n10\n1\n20\n2\n41\n2\n42\n2\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\nleaf\n70\n0\n10\n0\n20\n0\n",
            "0\n3DFACE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n0\n12\n2\n22\n1\n13\n0\n23\n1\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nfacets\n2\nouter\n10\n10\n20\n20\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(parsed.layers(), &["facets".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("nested 3DFACE must expand to one profile")
    };
    assert!(profile.closed());
    assert_eq!(profile.segments().len(), 4);
    assert_eq!(profile.segments()[0].start_mm(), [11.0, 22.0]);
    assert_eq!(profile.segments()[0].end_mm(), [11.0, 26.0]);
    assert_eq!(profile.segments()[1].end_mm(), [9.0, 26.0]);
    assert_eq!(profile.segments()[3].end_mm(), [11.0, 22.0]);
}

#[test]
fn invalid_non_planar_hidden_edge_and_ambiguous_3dfaces_fail_closed() {
    let options = DxfImportOptions::new(None);
    for (entity, expected) in [
        (
            "0\n3DFACE\n10\n0\n20\n0\n11\n1\n21\n0\n12\n0\n22\n1\n",
            DxfImportError::MalformedPairs,
        ),
        (
            "0\n3DFACE\n10\n0\n20\n0\n11\n1\n21\n0\n12\n0\n22\n1\n32\n1\n13\n0\n23\n1\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            "0\n3DFACE\n10\n0\n20\n0\n11\n1\n21\n0\n12\n0\n22\n1\n13\n0\n23\n1\n70\n1\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\n3DFACE\n10\n0\n20\n0\n11\n1\n21\n0\n12\n2\n22\n0\n13\n2\n23\n0\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            "0\n3DFACE\n10\n0\n20\n0\n11\n0\n21\n0\n12\n0\n22\n1\n13\n1\n23\n1\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            "0\n3DFACE\n10\n0\n20\n0\n11\n2\n21\n2\n12\n0\n22\n2\n13\n2\n23\n0\n",
            DxfImportError::AmbiguousGeometry,
        ),
    ] {
        assert_eq!(inspect_dxf(&dxf(Some(4), entity), options), Err(expected));
    }
}

#[test]
fn nested_dimension_graphics_blocks_preserve_exact_profiles_clone_offsets_and_layers() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n8\n0\n2\n*D2\n70\n1\n10\n0\n20\n0\n",
            "0\nDIMENSION\n8\n0\n2\n*D1\n3\nSTANDARD\n10\n0\n20\n0\n12\n1\n22\n2\n70\n32\n",
            "0\nENDBLK\n8\n0\n",
            "0\nBLOCK\n8\n0\n2\n*D1\n70\n1\n10\n0\n20\n0\n",
            "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n0\n",
            "0\nLINE\n8\ndetail\n10\n0\n20\n1\n11\n2\n21\n1\n",
            "0\nTEXT\n8\n0\n10\n1\n20\n0\n1\n2.00\n",
            "0\nENDBLK\n8\n0\n"
        ),
        "0\nDIMENSION\n8\ndimensions\n2\n*D2\n3\nSTANDARD\n10\n0\n20\n0\n12\n10\n22\n20\n70\n32\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(
        parsed.layers(),
        &["detail".to_owned(), "dimensions".to_owned()]
    );
    assert_eq!(parsed.profiles().len(), 2);
    let inherited = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "dimensions")
        .unwrap();
    assert!(matches!(
        inherited.segments(),
        [ProfileSegment::Line {
            start_mm: [11.0, 22.0],
            end_mm: [13.0, 22.0]
        }]
    ));
    let explicit = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "detail")
        .unwrap();
    assert!(matches!(
        explicit.segments(),
        [ProfileSegment::Line {
            start_mm: [11.0, 23.0],
            end_mm: [13.0, 23.0]
        }]
    ));
    for subject in ["*D1", "*D2"] {
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "dxf.dimension-semantics-dropped"
                && diagnostic.subject() == Some(subject)
                && diagnostic.count() == 1
        }));
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "dxf.dimension-graphics"
                && diagnostic.subject() == Some(subject)
                && diagnostic.count() == 1
        }));
    }
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("TEXT")
            && diagnostic.count() == 1
    }));
}

#[test]
fn malformed_undefined_empty_non_anonymous_and_non_planar_dimensions_fail_closed() {
    let valid_block = concat!(
        "0\nBLOCK\n2\n*D1\n70\n1\n10\n0\n20\n0\n",
        "0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n",
        "0\nENDBLK\n"
    );
    let options = DxfImportOptions::new(None);
    for (blocks, entity, expected) in [
        (
            valid_block,
            "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n",
            DxfImportError::MalformedPairs,
        ),
        (
            valid_block,
            "0\nDIMENSION\n2\nmissing\n10\n0\n20\n0\n70\n32\n",
            DxfImportError::InvalidBlock,
        ),
        (
            "0\nBLOCK\n2\n*D1\n70\n1\n10\n0\n20\n0\n0\nENDBLK\n",
            "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n70\n32\n",
            DxfImportError::InvalidBlock,
        ),
        (
            "0\nBLOCK\n2\nnamed\n70\n0\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDBLK\n",
            "0\nDIMENSION\n2\nnamed\n10\n0\n20\n0\n70\n32\n",
            DxfImportError::InvalidBlock,
        ),
        (
            "0\nBLOCK\n2\n*D1\n70\n1\n10\n1\n20\n0\n0\nLINE\n10\n1\n20\n0\n11\n2\n21\n0\n0\nENDBLK\n",
            "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n70\n32\n",
            DxfImportError::InvalidBlock,
        ),
        (
            valid_block,
            "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n12\n1\n70\n32\n",
            DxfImportError::MalformedPairs,
        ),
        (
            valid_block,
            "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n12\n1\n22\n2\n32\n1\n70\n32\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            valid_block,
            "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n12\n1000001\n22\n0\n70\n32\n",
            DxfImportError::CoordinateOutOfRange,
        ),
    ] {
        assert_eq!(
            inspect_dxf(&dxf_with_blocks(4, blocks, entity), options),
            Err(expected)
        );
    }

    let cycle = concat!(
        "0\nBLOCK\n2\n*D1\n70\n1\n10\n0\n20\n0\n",
        "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n70\n32\n",
        "0\nENDBLK\n"
    );
    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, cycle, "0\nDIMENSION\n2\n*D1\n10\n0\n20\n0\n70\n32\n"),
            options
        ),
        Err(DxfImportError::InvalidBlock)
    );
}

#[test]
fn translated_block_inserts_expand_exact_profiles_with_layer_inheritance_and_units() {
    let source = dxf_with_blocks(
        5,
        concat!(
            "0\nBLOCK\n8\n0\n2\n*Model_Space\n70\n0\n10\n0\n20\n0\n0\nENDBLK\n8\n0\n",
            "0\nBLOCK\n8\n0\n2\nSymbol\n3\nSymbol\n70\n0\n10\n1\n20\n2\n30\n0\n",
            "0\nLWPOLYLINE\n8\n0\n90\n3\n70\n1\n10\n1\n20\n2\n10\n3\n20\n2\n10\n1\n20\n4\n",
            "0\nCIRCLE\n8\ndetail\n10\n2\n20\n3\n40\n0.5\n",
            "0\nENDBLK\n8\n0\n"
        ),
        concat!(
            "0\nINSERT\n8\nalpha\n2\nsymbol\n10\n10\n20\n20\n",
            "0\nINSERT\n8\nbeta\n2\nSYMBOL\n10\n20\n20\n30\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(
        parsed.layers(),
        &["alpha".to_owned(), "beta".to_owned(), "detail".to_owned()]
    );
    assert_eq!(parsed.profiles().len(), 4);
    let alpha = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "alpha")
        .unwrap();
    assert!(alpha.closed());
    assert_eq!(alpha.segments()[0].start_mm(), [100.0, 200.0]);
    assert_eq!(alpha.segments()[0].end_mm(), [120.0, 200.0]);
    let beta = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "beta")
        .unwrap();
    assert_eq!(beta.segments()[0].start_mm(), [200.0, 300.0]);
    assert_eq!(
        parsed
            .profiles()
            .iter()
            .filter(|profile| profile.layer() == "detail")
            .count(),
        2
    );
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("symbol")
            && diagnostic.count() == 1
    }));
    assert!(!parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.section-ignored" && diagnostic.subject() == Some("BLOCKS")
    }));
}

#[test]
fn xref_external_and_unknown_block_flags_fail_closed() {
    for flags in [4, 8, 16, 32, 64, 128, -1] {
        let blocks = format!(
            "0\nBLOCK\n2\npart\n70\n{flags}\n10\n0\n20\n0\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n0\nENDBLK\n"
        );
        assert_eq!(
            inspect_dxf(
                &dxf_with_blocks(4, &blocks, "0\nINSERT\n2\npart\n10\n0\n20\n0\n"),
                DxfImportOptions::new(None)
            ),
            Err(DxfImportError::InvalidBlock)
        );
    }
}

#[test]
fn attributed_top_level_and_nested_inserts_keep_exact_geometry_and_report_losses() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n8\n0\n2\nouter\n70\n2\n10\n0\n20\n0\n",
            "0\nATTDEF\n8\nlabels\n2\nNAME\n1\nunnamed\n10\n0\n20\n0\n",
            "0\nATTDEF\n8\nlabels\n2\nCODE\n1\nnone\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\n*U1\n10\n1\n20\n2\n66\n1\n",
            "0\nATTRIB\n8\nlabels\n2\nPART\n1\ninside\n10\n1\n20\n2\n",
            "0\nSEQEND\n8\nlabels\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n8\n0\n2\n*U1\n70\n3\n10\n0\n20\n0\n",
            "0\nATTDEF\n8\nlabels\n2\nPART\n1\nunknown\n10\n0\n20\n0\n",
            "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n1\n",
            "0\nENDBLK\n"
        ),
        concat!(
            "0\nINSERT\n8\ninstances\n2\nouter\n10\n10\n20\n20\n41\n-2\n42\n3\n50\n90\n66\n1\n",
            "0\nATTRIB\n2\nNAME\n1\nfirst\n10\n10\n20\n20\n",
            "0\nATTRIB\n2\nCODE\n1\nA-01\n10\n10\n20\n20\n",
            "0\nSEQEND\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["instances".to_owned()]);
    let [profile] = parsed.profiles() else {
        panic!("attributed nested INSERT must retain one exact profile")
    };
    assert!(matches!(
        profile.segments(),
        [ProfileSegment::Line {
            start_mm: [4.0, 18.0],
            end_mm: [1.0, 14.0]
        }]
    ));
    for (subject, count) in [("*U1", 1), ("outer", 2)] {
        assert!(parsed.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "dxf.insert-attributes-dropped"
                && diagnostic.subject() == Some(subject)
                && diagnostic.count() == count
        }));
    }
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("ATTDEF")
            && diagnostic.count() == 3
    }));
}

#[test]
fn malformed_or_orphaned_insert_attribute_sequences_fail_closed() {
    let block = concat!(
        "0\nBLOCK\n2\npart\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n",
        "0\nENDBLK\n"
    );
    let options = DxfImportOptions::new(None);
    for entities in [
        "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n2\n",
        "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n",
        "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n0\nSEQEND\n",
        "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n0\nATTRIB\n0\nSEQEND\n",
        "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n0\nATTRIB\n2\nTAG\n2\nOTHER\n1\nvalue\n0\nSEQEND\n",
        "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n0\nATTRIB\n2\nTAG\n1\nvalue\n0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n",
        "0\nATTRIB\n2\nTAG\n1\nvalue\n0\nINSERT\n2\npart\n10\n0\n20\n0\n",
        "0\nSEQEND\n0\nINSERT\n2\npart\n10\n0\n20\n0\n",
    ] {
        assert_eq!(
            inspect_dxf(&dxf_with_blocks(4, block, entities), options),
            Err(DxfImportError::MalformedPairs)
        );
    }
}

#[test]
fn insert_attribute_children_share_the_global_entity_budget() {
    let block = concat!(
        "0\nBLOCK\n2\npart\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n",
        "0\nENDBLK\n"
    );
    let mut entities = String::from("0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n");
    for _ in 0..9_996 {
        entities.push_str("0\nATTRIB\n2\nTAG\n1\nvalue\n");
    }
    entities.push_str("0\nSEQEND\n");

    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, block, &entities),
            DxfImportOptions::new(None)
        ),
        Err(DxfImportError::TooManyEntities)
    );
}

#[test]
fn rotated_uniformly_scaled_inserts_transform_about_the_block_base_point() {
    let source = dxf_with_blocks(
        5,
        concat!(
            "0\nBLOCK\n8\n0\n2\nrotated\n70\n0\n10\n1\n20\n2\n30\n0\n",
            "0\nLINE\n8\n0\n10\n1\n20\n2\n11\n3\n21\n2\n",
            "0\nCIRCLE\n8\ndetail\n10\n2\n20\n3\n40\n0.5\n",
            "0\nENDBLK\n8\n0\n"
        ),
        "0\nINSERT\n8\ninstances\n2\nrotated\n10\n10\n20\n20\n41\n2\n42\n2\n50\n90\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(
        parsed.layers(),
        &["detail".to_owned(), "instances".to_owned()]
    );
    assert_eq!(parsed.profiles().len(), 2);
    let line = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "instances")
        .unwrap();
    assert!(matches!(
        line.segments(),
        [ProfileSegment::Line {
            start_mm: [100.0, 200.0],
            end_mm: [100.0, 240.0]
        }]
    ));
    let circle = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "detail")
        .unwrap();
    assert!(circle.closed());
    assert!(matches!(
        circle.segments(),
        [
            ProfileSegment::CircularArc {
                start_mm: [80.0, 230.0],
                end_mm: [80.0, 210.0],
                center_mm: [80.0, 220.0],
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [80.0, 210.0],
                end_mm: [80.0, 230.0],
                center_mm: [80.0, 220.0],
                clockwise: false,
            }
        ]
    ));
}

#[test]
fn rotated_non_uniform_line_only_inserts_and_arrays_remain_exact() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n8\n0\n2\nplate\n70\n0\n10\n1\n20\n2\n30\n0\n",
            "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n1\n20\n2\n10\n3\n20\n2\n10\n3\n20\n3\n10\n1\n20\n3\n",
            "0\nENDBLK\n8\n0\n"
        ),
        "0\nINSERT\n8\ninstances\n2\nplate\n10\n10\n20\n20\n41\n2\n42\n3\n50\n90\n70\n2\n44\n5\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["instances".to_owned()]);
    assert_eq!(parsed.profiles().len(), 2);
    let boundaries = parsed
        .profiles()
        .iter()
        .map(|profile| {
            assert!(profile.closed());
            profile
                .segments()
                .iter()
                .map(|segment| (segment.start_mm(), segment.end_mm()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        boundaries,
        vec![
            vec![
                ([10.0, 20.0], [10.0, 24.0]),
                ([10.0, 24.0], [7.0, 24.0]),
                ([7.0, 24.0], [7.0, 20.0]),
                ([7.0, 20.0], [10.0, 20.0]),
            ],
            vec![
                ([10.0, 25.0], [10.0, 29.0]),
                ([10.0, 29.0], [7.0, 29.0]),
                ([7.0, 29.0], [7.0, 25.0]),
                ([7.0, 25.0], [10.0, 25.0]),
            ],
        ]
    );
}

#[test]
fn reflected_non_uniform_line_only_inserts_and_arrays_remain_exact() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n8\n0\n2\nplate\n70\n0\n10\n1\n20\n2\n30\n0\n",
            "0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n1\n20\n2\n10\n3\n20\n2\n10\n3\n20\n3\n10\n1\n20\n3\n",
            "0\nENDBLK\n8\n0\n"
        ),
        "0\nINSERT\n8\ninstances\n2\nplate\n10\n10\n20\n20\n41\n-2\n42\n3\n50\n90\n70\n2\n44\n5\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["instances".to_owned()]);
    let boundaries = parsed
        .profiles()
        .iter()
        .map(|profile| {
            assert!(profile.closed());
            profile
                .segments()
                .iter()
                .map(|segment| (segment.start_mm(), segment.end_mm()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        boundaries,
        vec![
            vec![
                ([10.0, 20.0], [10.0, 16.0]),
                ([10.0, 16.0], [7.0, 16.0]),
                ([7.0, 16.0], [7.0, 20.0]),
                ([7.0, 20.0], [10.0, 20.0]),
            ],
            vec![
                ([10.0, 25.0], [10.0, 21.0]),
                ([10.0, 21.0], [7.0, 21.0]),
                ([7.0, 21.0], [7.0, 25.0]),
                ([7.0, 25.0], [10.0, 25.0]),
            ],
        ]
    );
}

#[test]
fn reflected_equal_magnitude_curves_flip_orientation_exactly() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n8\n0\n2\nring\n70\n0\n10\n0\n20\n0\n30\n0\n",
            "0\nCIRCLE\n8\n0\n10\n2\n20\n0\n40\n1\n",
            "0\nENDBLK\n8\n0\n"
        ),
        concat!(
            "0\nINSERT\n8\nmirror\n2\nring\n10\n10\n20\n20\n41\n-2\n42\n2\n50\n90\n",
            "0\nINSERT\n8\nturn\n2\nring\n10\n30\n20\n20\n41\n-2\n42\n-2\n"
        ),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    let mirror = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "mirror")
        .unwrap();
    assert!(matches!(
        mirror.segments(),
        [
            ProfileSegment::CircularArc {
                start_mm: [10.0, 14.0],
                end_mm: [10.0, 18.0],
                center_mm: [10.0, 16.0],
                clockwise: true,
            },
            ProfileSegment::CircularArc {
                start_mm: [10.0, 18.0],
                end_mm: [10.0, 14.0],
                center_mm: [10.0, 16.0],
                clockwise: true,
            }
        ]
    ));
    let turn = parsed
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "turn")
        .unwrap();
    assert!(turn.segments().iter().all(|segment| matches!(
        segment,
        ProfileSegment::CircularArc {
            center_mm: [26.0, 20.0],
            clockwise: false,
            ..
        }
    )));
}

#[test]
fn nested_non_uniform_line_only_insert_composes_with_outer_transform() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
            "0\nINSERT\n8\n0\n2\ninner\n10\n1\n20\n2\n41\n2\n42\n3\n50\n90\n",
            "0\nENDBLK\n",
            "0\nBLOCK\n2\ninner\n70\n0\n10\n0\n20\n0\n",
            "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n1\n",
            "0\nENDBLK\n"
        ),
        "0\nINSERT\n8\nnested\n2\nouter\n10\n10\n20\n20\n41\n0.5\n42\n0.5\n50\n-90\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    let [profile] = parsed.profiles() else {
        panic!("nested non-uniform line-only INSERT must produce one profile")
    };
    assert_eq!(profile.layer(), "nested");
    assert!(matches!(
        profile.segments(),
        [ProfileSegment::Line {
            start_mm: [11.0, 19.5],
            end_mm: [13.0, 21.0]
        }]
    ));
}

#[test]
fn minsert_arrays_expand_row_major_with_rotated_unscaled_spacing() {
    let source = dxf_with_blocks(
        5,
        concat!(
            "0\nBLOCK\n8\n0\n2\narray-part\n70\n0\n10\n1\n20\n2\n30\n0\n",
            "0\nLINE\n8\n0\n10\n1\n20\n2\n11\n3\n21\n2\n",
            "0\nENDBLK\n8\n0\n"
        ),
        "0\nINSERT\n8\ninstances\n2\narray-part\n10\n10\n20\n20\n41\n2\n42\n2\n50\n90\n70\n2\n44\n3\n71\n3\n45\n4\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["instances".to_owned()]);
    assert_eq!(parsed.profiles().len(), 6);
    let endpoints = parsed
        .profiles()
        .iter()
        .map(|profile| {
            assert_eq!(profile.layer(), "instances");
            assert!(!profile.closed());
            let [segment] = profile.segments() else {
                panic!("each expanded array profile must retain one line")
            };
            (segment.start_mm(), segment.end_mm())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        endpoints,
        vec![
            ([100.0, 200.0], [100.0, 240.0]),
            ([100.0, 230.0], [100.0, 270.0]),
            ([60.0, 200.0], [60.0, 240.0]),
            ([60.0, 230.0], [60.0, 270.0]),
            ([20.0, 200.0], [20.0, 240.0]),
            ([20.0, 230.0], [20.0, 270.0]),
        ]
    );
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("array-part")
            && diagnostic.count() == 6
    }));
}

#[test]
fn forward_referenced_nested_inserts_compose_exact_transforms_arrays_and_layer_inheritance() {
    let source = dxf_with_blocks(
        4,
        concat!(
            "0\nBLOCK\n8\n0\n2\nouter\n70\n0\n10\n10\n20\n20\n",
            "0\nINSERT\n8\n0\n2\nchild\n10\n12\n20\n24\n41\n2\n42\n2\n50\n90\n70\n2\n44\n5\n",
            "0\nENDBLK\n8\n0\n",
            "0\nBLOCK\n8\n0\n2\nchild\n70\n0\n10\n1\n20\n2\n",
            "0\nLINE\n8\n0\n10\n1\n20\n2\n11\n3\n21\n2\n",
            "0\nENDBLK\n8\n0\n"
        ),
        "0\nINSERT\n8\ninstances\n2\nouter\n10\n100\n20\n200\n41\n0.5\n42\n0.5\n50\n-90\n",
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();

    assert_eq!(ketchup_core::import::DXF_PARSER_VERSION, "27");
    assert_eq!(parsed.layers(), &["instances".to_owned()]);
    assert_eq!(parsed.profiles().len(), 2);
    let endpoints = parsed
        .profiles()
        .iter()
        .map(|profile| {
            assert_eq!(profile.layer(), "instances");
            let [segment] = profile.segments() else {
                panic!("each nested array placement must retain one line")
            };
            (segment.start_mm(), segment.end_mm())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        endpoints,
        vec![
            ([102.0, 199.0], [104.0, 199.0]),
            ([104.5, 199.0], [106.5, 199.0]),
        ]
    );
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("child")
            && diagnostic.count() == 2
    }));
    assert!(parsed.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("outer")
            && diagnostic.count() == 1
    }));
}

#[test]
fn cyclic_or_excessively_deep_nested_block_graphs_fail_closed() {
    let options = DxfImportOptions::new(None);
    let self_cycle = concat!(
        "0\nBLOCK\n2\nself\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n2\nself\n10\n0\n20\n0\n",
        "0\nENDBLK\n"
    );
    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, self_cycle, "0\nINSERT\n2\nself\n10\n0\n20\n0\n"),
            options
        ),
        Err(DxfImportError::InvalidBlock)
    );

    let mutual_cycle = concat!(
        "0\nBLOCK\n2\na\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n2\nb\n10\n0\n20\n0\n",
        "0\nENDBLK\n",
        "0\nBLOCK\n2\nb\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n2\na\n10\n0\n20\n0\n",
        "0\nENDBLK\n"
    );
    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, mutual_cycle, "0\nINSERT\n2\na\n10\n0\n20\n0\n"),
            options
        ),
        Err(DxfImportError::InvalidBlock)
    );

    let mut too_deep = String::new();
    for index in 0..33 {
        too_deep.push_str(&format!("0\nBLOCK\n2\nd{index}\n70\n0\n10\n0\n20\n0\n"));
        if index == 32 {
            too_deep.push_str("0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n");
        } else {
            too_deep.push_str(&format!("0\nINSERT\n2\nd{}\n10\n0\n20\n0\n", index + 1));
        }
        too_deep.push_str("0\nENDBLK\n");
    }
    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, &too_deep, "0\nINSERT\n2\nd0\n10\n0\n20\n0\n"),
            options
        ),
        Err(DxfImportError::InvalidBlock)
    );
}

#[test]
fn invalid_or_unsupported_block_inserts_fail_closed() {
    let valid_block = concat!(
        "0\nBLOCK\n2\npart\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n",
        "0\nENDBLK\n"
    );
    let curved_block = concat!(
        "0\nBLOCK\n2\npart\n70\n0\n10\n0\n20\n0\n",
        "0\nCIRCLE\n8\n0\n10\n0\n20\n0\n40\n1\n",
        "0\nENDBLK\n"
    );
    let options = DxfImportOptions::new(None);
    for (blocks, entities, expected) in [
        (
            "",
            "0\nINSERT\n2\nmissing\n10\n0\n20\n0\n",
            DxfImportError::InvalidBlock,
        ),
        (
            "0\nBLOCK\n2\nempty\n70\n0\n10\n0\n20\n0\n0\nENDBLK\n",
            "0\nINSERT\n2\nempty\n10\n0\n20\n0\n",
            DxfImportError::InvalidBlock,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n41\n0\n42\n-1\n",
            DxfImportError::UnsupportedInsertTransform,
        ),
        (
            curved_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n41\n2\n",
            DxfImportError::UnsupportedInsertTransform,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n43\n2\n",
            DxfImportError::UnsupportedInsertTransform,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n70\n2\n",
            DxfImportError::AmbiguousGeometry,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n70\n0\n44\n1\n",
            DxfImportError::UnsupportedInsertTransform,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n70\n171\n44\n1\n",
            DxfImportError::TooManyProfiles,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n70\n10001\n44\n1\n",
            DxfImportError::TooManyEntities,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n999999\n20\n0\n70\n2\n44\n2\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n70\n2\n44\nNaN\n",
            DxfImportError::InvalidNumber,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n66\n1\n",
            DxfImportError::MalformedPairs,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n30\n1\n",
            DxfImportError::NonPlanarGeometry,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n999999\n20\n0\n41\n2\n42\n2\n",
            DxfImportError::CoordinateOutOfRange,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n0\n20\n0\n41\n0.000000000001\n42\n0.000000000001\n",
            DxfImportError::DegenerateGeometry,
        ),
        (
            valid_block,
            "0\nINSERT\n2\npart\n10\n1000001\n20\n0\n",
            DxfImportError::CoordinateOutOfRange,
        ),
    ] {
        assert_eq!(
            inspect_dxf(&dxf_with_blocks(4, blocks, entities), options),
            Err(expected)
        );
    }

    let duplicate = format!("{valid_block}{}", valid_block.replace("part", "PART"));
    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, &duplicate, "0\nINSERT\n2\npart\n10\n0\n20\n0\n"),
            options
        ),
        Err(DxfImportError::InvalidBlock)
    );
    let nested = concat!(
        "0\nBLOCK\n2\nouter\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n2\ninner\n10\n0\n20\n0\n",
        "0\nENDBLK\n"
    );
    assert_eq!(
        inspect_dxf(
            &dxf_with_blocks(4, nested, "0\nINSERT\n2\nouter\n10\n0\n20\n0\n"),
            options
        ),
        Err(DxfImportError::InvalidBlock)
    );
}

#[test]
fn block_and_entity_streams_share_one_global_entity_budget() {
    let mut blocks = String::new();
    for index in 0..5_000 {
        blocks.push_str(&format!(
            "0\nBLOCK\n2\nempty-{index}\n70\n0\n10\n0\n20\n0\n0\nENDBLK\n"
        ));
    }
    let source = dxf_with_blocks(4, &blocks, "0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n");
    assert_eq!(
        inspect_dxf(&source, DxfImportOptions::new(None)),
        Err(DxfImportError::TooManyEntities)
    );
}

#[test]
fn unitless_files_require_review_and_scale_user_units_before_validation() {
    let source = dxf(None, "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n");
    assert_eq!(
        inspect_dxf(&source, DxfImportOptions::new(None)),
        Err(DxfImportError::UnitsRequired)
    );

    let parsed = inspect_dxf(&source, DxfImportOptions::new(Some(ImportLengthUnit::Inch))).unwrap();
    assert_eq!(
        parsed.units().authority(),
        ImportUnitAuthority::UserDeclared
    );
    assert_eq!(parsed.profiles()[0].segments()[0].end_mm(), [25.4, 0.0]);
}

#[test]
fn malformed_non_planar_and_ambiguous_geometry_fail_closed_with_typed_errors() {
    let malformed = b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n1\n";
    assert_eq!(
        inspect_dxf(
            malformed,
            DxfImportOptions::new(Some(ImportLengthUnit::Millimetre))
        ),
        Err(DxfImportError::MalformedSections)
    );

    let non_planar = dxf(
        Some(4),
        "0\nLINE\n10\n0\n20\n0\n30\n1\n11\n1\n21\n0\n31\n1\n",
    );
    assert_eq!(
        inspect_dxf(&non_planar, DxfImportOptions::new(None)),
        Err(DxfImportError::NonPlanarGeometry)
    );

    let branch = dxf(
        Some(4),
        concat!(
            "0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n",
            "0\nLINE\n10\n0\n20\n0\n11\n0\n21\n1\n",
            "0\nLINE\n10\n0\n20\n0\n11\n-1\n21\n0\n"
        ),
    );
    assert_eq!(
        inspect_dxf(&branch, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );

    let duplicate_profile = dxf(
        Some(4),
        concat!(
            "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n",
            "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n10\n0\n20\n0\n10\n1\n20\n0\n"
        ),
    );
    assert_eq!(
        inspect_dxf(&duplicate_profile, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );

    let duplicate_bulge = dxf(
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n10\n0\n20\n0\n42\n0\n42\n0\n10\n1\n20\n0\n",
    );
    assert_eq!(
        inspect_dxf(&duplicate_bulge, DxfImportOptions::new(None)),
        Err(DxfImportError::MalformedPairs)
    );

    let equivalent_arc_and_bulge = dxf(
        Some(4),
        concat!(
            "0\nARC\n8\n0\n10\n0\n20\n0\n40\n1\n50\n0\n51\n45\n",
            "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n",
            "10\n1\n20\n0\n42\n0.198912367379658\n",
            "10\n0.7071067811865476\n20\n0.7071067811865475\n"
        ),
    );
    assert_eq!(
        inspect_dxf(&equivalent_arc_and_bulge, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );

    let duplicate_arc = dxf(
        Some(4),
        concat!(
            "0\nARC\n8\n0\n10\n0\n20\n0\n40\n1\n50\n17\n51\n213\n",
            "0\nARC\n8\n0\n10\n0\n20\n0\n40\n1\n50\n17\n51\n213\n"
        ),
    );
    assert_eq!(
        inspect_dxf(&duplicate_arc, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );

    let duplicate_polyline_edge = dxf(
        Some(4),
        concat!(
            "0\nLWPOLYLINE\n8\n0\n90\n3\n70\n0\n10\n0\n20\n0\n10\n1\n20\n0\n10\n2\n20\n0\n",
            "0\nLWPOLYLINE\n8\n0\n90\n3\n70\n0\n10\n0\n20\n0\n10\n1\n20\n0\n10\n1\n20\n1\n"
        ),
    );
    assert_eq!(
        inspect_dxf(&duplicate_polyline_edge, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );

    let closed_two_vertex_polyline = dxf(
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n1\n10\n0\n20\n0\n10\n1\n20\n0\n",
    );
    assert_eq!(
        inspect_dxf(&closed_two_vertex_polyline, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );

    let unsupported_only = dxf(
        Some(4),
        "0\nSPLINE\n8\n0\n70\n8\n71\n2\n72\n6\n73\n3\n74\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n10\n0\n20\n0\n10\n1\n20\n1\n10\n2\n20\n0\n",
    );
    assert_eq!(
        inspect_dxf(&unsupported_only, DxfImportOptions::new(None)),
        Err(DxfImportError::NoSupportedGeometry)
    );
}

#[test]
fn arc_and_bulge_sweeps_must_remain_inside_the_coordinate_envelope() {
    let options = DxfImportOptions::new(None);
    let outward_arc = dxf(
        Some(4),
        "0\nARC\n8\n0\n10\n999999.5\n20\n0\n40\n1\n50\n270\n51\n90\n",
    );
    assert_eq!(
        inspect_dxf(&outward_arc, options),
        Err(DxfImportError::CoordinateOutOfRange)
    );
    let inward_arc = dxf(
        Some(4),
        "0\nARC\n8\n0\n10\n999999.5\n20\n0\n40\n1\n50\n90\n51\n270\n",
    );
    assert!(inspect_dxf(&inward_arc, options).is_ok());

    for (x, bulge) in [(999999.5, 1.0), (-999999.5, -1.0), (999998.5, 2.0)] {
        let source = dxf(
            Some(4),
            &format!(
                "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n10\n{x}\n20\n-1\n42\n{bulge}\n10\n{x}\n20\n1\n"
            ),
        );
        assert_eq!(
            inspect_dxf(&source, options),
            Err(DxfImportError::CoordinateOutOfRange)
        );
    }
    let inward_bulge = dxf(
        Some(4),
        "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n10\n999999.5\n20\n-1\n42\n-1\n10\n999999.5\n20\n1\n",
    );
    assert!(inspect_dxf(&inward_bulge, options).is_ok());
}

#[test]
fn loose_line_arc_joins_are_bounded_deterministic_and_fail_on_ambiguity() {
    let options = DxfImportOptions::new(None);
    let arc = "0\nARC\n8\n0\n10\n0\n20\n0\n40\n1\n50\n0\n51\n45\n";
    let line = "0\nLINE\n8\n0\n10\n0.7071067811865476\n20\n0.7071067811865475\n11\n2\n21\n2\n";
    let forward = dxf(Some(4), &format!("{arc}{line}"));
    let reversed = dxf(
        Some(4),
        &format!(
            "0\nLINE\n8\n0\n10\n2\n20\n2\n11\n0.7071067811865476\n21\n0.7071067811865475\n{arc}"
        ),
    );
    let parsed = inspect_dxf(&forward, options).unwrap();
    let repeated = inspect_dxf(&reversed, options).unwrap();
    assert_eq!(parsed.profiles(), repeated.profiles());
    assert_eq!(parsed.profiles().len(), 1);
    let segments = parsed.profiles()[0].segments();
    assert_eq!(segments[0].end_mm(), segments[1].start_mm());

    let separated = dxf(
        Some(4),
        &format!("{arc}0\nLINE\n8\n0\n10\n0.7071067832\n20\n0.7071067811865475\n11\n2\n21\n2\n"),
    );
    assert_eq!(
        inspect_dxf(&separated, options).unwrap().profiles().len(),
        2
    );

    let ambiguous = dxf(
        Some(4),
        concat!(
            "0\nARC\n8\n0\n10\n0\n20\n0\n40\n1\n50\n270\n51\n0\n",
            "0\nARC\n8\n0\n10\n0\n20\n0.0000000005\n40\n1\n50\n270\n51\n0\n",
            "0\nLINE\n8\n0\n10\n1\n20\n0.00000000025\n11\n2\n21\n0\n"
        ),
    );
    assert_eq!(
        inspect_dxf(&ambiguous, options),
        Err(DxfImportError::AmbiguousGeometry)
    );
}

#[test]
fn dxf_plan_is_deterministic_one_step_undoable_and_persistent() {
    let source = supported_fixture();
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let options = DxfImportOptions::new(None);
    let maximum_profiles = disconnected_lines(170);
    let maximum_batch = plan_dxf_import(
        &document.current(),
        &maximum_profiles,
        "maximum-profiles.dxf",
        options,
    )
    .unwrap();
    document.prepare_proposal(maximum_batch).unwrap();
    assert_eq!(
        inspect_dxf(&disconnected_lines(171), options),
        Err(DxfImportError::TooManyProfiles)
    );

    let batch = plan_dxf_import(&document.current(), &source, "profiles.dxf", options).unwrap();
    let repeated = plan_dxf_import(&document.current(), &source, "profiles.dxf", options).unwrap();
    assert_eq!(batch.digest(), repeated.digest());

    let small_arc = dxf(
        Some(4),
        "0\nARC\n8\n0\n10\n0\n20\n0\n40\n0.1\n50\n1.5\n51\n18.5\n",
    );
    let small_arc_batch =
        plan_dxf_import(&document.current(), &small_arc, "small-arc.dxf", options).unwrap();
    document.prepare_proposal(small_arc_batch).unwrap();

    let asymmetric_bulge = dxf(
        Some(4),
        concat!(
            "0\nLWPOLYLINE\n8\n0\n90\n2\n70\n0\n",
            "10\n0.6978555372935438\n20\n-0.06761611340167839\n42\n1.2818444289480573\n",
            "10\n0.7061081813638013\n20\n-0.08431497132046811\n"
        ),
    );
    let asymmetric_bulge_batch = plan_dxf_import(
        &document.current(),
        &asymmetric_bulge,
        "asymmetric-bulge.dxf",
        options,
    )
    .unwrap();
    document.prepare_proposal(asymmetric_bulge_batch).unwrap();

    let proposal = document.prepare_proposal(batch).unwrap();
    assert_eq!(document.current().canonical_digest(), before);
    document.commit_verified_proposal(&proposal).unwrap();
    let committed = document.current();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(committed.import_receipts().count(), 1);
    assert_eq!(
        committed
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::SegmentProfile { .. }))
            .count(),
        2
    );
    let receipt = committed.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::Dxf);
    assert_eq!(
        receipt.parser_version(),
        ketchup_core::import::DXF_PARSER_VERSION
    );
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.source_byte_len(), source.len() as u64);
    assert_eq!(receipt.outputs().len(), 6);

    let committed_digest = committed.canonical_digest();
    let encoded = persistence::save(&committed);
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}
