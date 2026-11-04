//! MECH-10(a): a raster image — an orthophoto — imports as an underlay, put where the
//! coordinates of its world file say, beside a drawing in the same coordinates.

use super::*;

/// A PNG of 4 × 2 pixels: its top row red, its bottom row blue.
const RED_OVER_BLUE_PNG: [u8; 74] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xf0, 0xca, 0xea,
    0x34, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x38, 0x21, 0x27, 0x07,
    0x47, 0x0c, 0x48, 0xec, 0x13, 0x00, 0x69, 0xba, 0x08, 0x21, 0x58, 0x8e, 0xd3, 0xc4, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn import_underlay(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: true,
    }
}

/// A cadastral line in S-JTSK metres, 10 m along the south edge of where the
/// orthophoto lies, imports first and is moved near the origin; the orthophoto, 4 × 2
/// pixels of 2.5 m whose world file puts it over that line, then lies on it: moved by the
/// same offset, its south corners at the line's ends, in one more Undo step and as no part.
#[test]
fn an_orthophoto_with_its_world_file_lies_on_the_drawing_in_the_same_coordinates() {
    let directory = tempfile::tempdir().unwrap();
    let site = directory.path().join("site.dxf");
    std::fs::write(
        &site,
        concat!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n6\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n",
            "0\nLINE\n8\nparcel\n10\n-500000\n20\n-1200000\n11\n-499990\n21\n-1200000\n",
            "0\nENDSEC\n0\nEOF\n",
        ),
    )
    .unwrap();
    let ortho = directory.path().join("ortofoto.png");
    std::fs::write(&ortho, RED_OVER_BLUE_PNG).unwrap();
    // Pixel size along x and y, the rotation terms, and the centre of the upper-left
    // pixel: the image covers x −500000…−499990 and y −1200000…−1199995 metres.
    std::fs::write(
        directory.path().join("ortofoto.pgw"),
        "2.5\n0.0\n0.0\n-2.5\n-499998.75\n-1199996.25\n",
    )
    .unwrap();
    let (mut app, mut bridge) = setup();
    let cadastre = bridge
        .execute(&mut app, import_underlay(&site), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let offset: [f64; 2] =
        serde_json::from_value(cadastre["underlay"]["offset_mm"].clone()).unwrap();
    let (undo, occurrences) = (
        app.undo_step_count(),
        app.document.current().occurrences().count(),
    );

    let answer = bridge
        .execute(&mut app, import_underlay(&ortho), false)
        .unwrap_or_else(|code| {
            panic!(
                "the orthophoto must import as an underlay: {code}: {}",
                app.action_digest()
            )
        });

    assert_eq!(answer["imported"], true, "{answer}");
    let raster = &answer["underlay"]["raster"];
    assert_eq!(raster["width_px"], 4, "{answer}");
    assert_eq!(raster["height_px"], 2, "{answer}");
    assert_eq!(raster["pixel_mm"], json!([2_500.0, 2_500.0]), "{answer}");
    assert_eq!(raster["world_file"], true, "{answer}");
    // An image is no drawing: neither the answer nor the digest names DXF.
    for said in [answer.to_string(), app.action_digest().to_string()] {
        assert!(!said.to_ascii_uppercase().contains("DXF"), "{said}");
    }
    assert!(
        app.action_digest().contains("image underlay"),
        "{}",
        app.action_digest()
    );
    assert_eq!(
        answer["underlay"]["offset_mm"],
        json!(offset),
        "moved as the drawing in the same coordinates was"
    );
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    assert_eq!(app.document.current().occurrences().count(), occurrences);

    let summary = bridge
        .execute(&mut app, Request::Summary {}, false)
        .unwrap();
    let in_document = |[x, y]: [f64; 2]| json!([x * 1_000.0 - offset[0], y * 1_000.0 - offset[1]]);
    assert_eq!(
        summary["underlays"][1]["raster"]["corners_mm"],
        json!([
            in_document([-500_000.0, -1_199_995.0]),
            in_document([-499_990.0, -1_199_995.0]),
            in_document([-499_990.0, -1_200_000.0]),
            in_document([-500_000.0, -1_200_000.0]),
        ]),
        "upper left, upper right, lower right, lower left: {summary}"
    );
    assert_eq!(summary["underlays"][0]["raster"], Value::Null, "{summary}");
}

/// A file named `.png` that holds no image is refused as the image it claims to be: the
/// refusal and the digest name an image, never a DXF drawing.
#[test]
fn a_broken_image_is_refused_as_an_image_not_as_a_drawing() {
    use crate::ImportKind;
    use ketchup_model::underlay::raster::RasterFormat;
    let directory = tempfile::tempdir().unwrap();
    let broken = directory.path().join("ortofoto.png");
    std::fs::write(&broken, b"no image here").unwrap();
    let (mut app, mut bridge) = setup();

    let code = bridge
        .execute(&mut app, import_underlay(&broken), false)
        .unwrap_err();
    let details = take_error_details();

    assert_eq!(code, "import_failed", "{details:?}");
    let details = details.unwrap();
    // Why it was refused; `fix_hint` is the one hint of every failed import.
    let why = json!([details["reason"], details["causes"]]).to_string();
    assert!(why.contains("PNG"), "{details}");
    assert!(!why.to_ascii_uppercase().contains("DXF"), "{details}");
    let error = crate::app::underlay::read_underlay_file(
        &broken,
        None,
        None,
        &ketchup_program::CancelToken::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.format, ImportKind::Image(RasterFormat::Png));
    app.report_import_failure(&broken, &error);
    let digest = app.action_digest().to_string();
    assert!(!digest.to_ascii_uppercase().contains("DXF"), "{digest}");
    assert!(digest.contains("Image underlay import"), "{digest}");
}
