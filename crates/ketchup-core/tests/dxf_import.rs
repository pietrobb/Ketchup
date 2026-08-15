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
            "0\nCIRCLE\n8\nunsupported\n10\n0\n20\n0\n40\n5\n"
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
                && diagnostic.subject() == Some("CIRCLE"))
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

    let unsupported_only = dxf(Some(4), "0\nSPLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n");
    assert_eq!(
        inspect_dxf(&unsupported_only, DxfImportOptions::new(None)),
        Err(DxfImportError::NoSupportedGeometry)
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
