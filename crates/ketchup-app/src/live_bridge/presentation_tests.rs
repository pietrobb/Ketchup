//! A client saves a presentation image of the model (Jožinko feedback U5): its background,
//! size and the see-through materials it names, never over an existing file.

use super::*;
use ketchup_model::document::{ClassificationCategoryId, ClassificationDimensionId};

const MEMBRANE: &str = "Membrane";

/// A window with the first box and a second one beside it of the material "Membrane".
fn window() -> (KetchupApp, LiveBridge, tempfile::TempDir) {
    let (mut app, bridge) = setup();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: crate::INITIAL_BOX_DEFINITION,
                name: "Membrane panel".to_owned(),
                transform: Transform::from_translation(60.0, 0.0, 0.0).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(210),
                name: ketchup_manufacturing::fabrication::MATERIAL_DIMENSION_V1.to_owned(),
                categories: vec![(ClassificationCategoryId(211), MEMBRANE.to_owned())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(2),
                dimension_id: ClassificationDimensionId(210),
                category_id: Some(ClassificationCategoryId(211)),
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    (app, bridge, tempfile::tempdir().unwrap())
}

fn presentation(path: &Path, image: Value) -> Request {
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: "presentation_png".to_owned(),
        drawing_scale: None,
        image: Some(serde_json::from_value(image).unwrap()),
        overwrite: false,
    }
}

/// The pixels of a PNG file as RGBA rows from the top, and its size.
fn pixels(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    (info.width, info.height, rgba)
}

#[test]
fn a_client_saves_a_presentation_image_with_its_background_size_and_see_through_material() {
    let (mut app, mut bridge, directory) = window();
    let digest = app.document.current().canonical_digest();
    let small = json!({"width": 96, "height": 64, "shadows": false, "ambient_occlusion": false});

    let opaque = directory.path().join("opaque.png");
    let mut image = small.clone();
    image["background"] = json!("transparent");
    let answer = bridge
        .execute(&mut app, presentation(&opaque, image), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["exported"], true, "{answer}");
    let (width, height, rgba) = pixels(&opaque);
    assert_eq!((width, height), (96, 64));
    assert_eq!(
        rgba[3], 0,
        "a transparent background leaves the corner clear"
    );
    assert!(
        rgba.chunks(4).any(|pixel| pixel[3] == u8::MAX),
        "the model is drawn opaque"
    );

    let see_through = directory.path().join("membrane.png");
    let mut image = small.clone();
    image["background"] = json!("#204060");
    image["opacity_percent"] = json!({MEMBRANE: 30});
    bridge
        .execute(&mut app, presentation(&see_through, image), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let (_, _, coloured) = pixels(&see_through);
    assert_eq!(&coloured[..4], &[0x20, 0x40, 0x60, u8::MAX]);

    let solid = directory.path().join("solid.png");
    let mut image = small;
    image["background"] = json!("#204060");
    bridge
        .execute(&mut app, presentation(&solid, image), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_ne!(
        pixels(&solid).2,
        coloured,
        "the membrane at 30 % lets the background through"
    );
    assert_eq!(app.document.current().canonical_digest(), digest);
}

#[test]
fn a_client_adds_dimensions_part_names_and_its_own_labels_to_the_image() {
    let (mut app, mut bridge, directory) = window();
    let base = json!({"width": 320, "height": 240, "shadows": false, "ambient_occlusion": false});
    let save = |app: &mut KetchupApp, bridge: &mut LiveBridge, name: &str, extra: Value| {
        let mut image = base.clone();
        for (key, value) in extra.as_object().unwrap() {
            image[key] = value.clone();
        }
        let path = directory.path().join(name);
        bridge
            .execute(app, presentation(&path, image), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        pixels(&path).2
    };
    let plain = save(&mut app, &mut bridge, "plain.png", json!({}));
    let dimensioned = save(
        &mut app,
        &mut bridge,
        "dimensions.png",
        json!({"dimensions": true}),
    );
    let named = save(
        &mut app,
        &mut bridge,
        "names.png",
        json!({"part_labels": true}),
    );
    let labelled = save(
        &mut app,
        &mut bridge,
        "label.png",
        json!({"labels": [{"text": "PVC membrane 0.8 mm", "at_mm": [60, 0, 0]}]}),
    );
    // Dark grey ink the plain image does not have.
    let ink = |rgba: &[u8]| {
        rgba.chunks(4)
            .filter(|p| p[0] < 90 && p[1] < 90 && p[2] < 90 && p[0].abs_diff(p[2]) < 12)
            .count()
    };
    assert!(ink(&dimensioned) > ink(&plain) + 100, "dimensions drawn");
    assert_ne!(named, plain, "part names drawn");
    assert_ne!(labelled, plain, "the client's label drawn");
    assert_ne!(labelled, named);
}

#[test]
fn a_client_presentation_never_replaces_a_file_and_names_what_it_cannot_draw() {
    let (mut app, mut bridge, directory) = window();
    let small = json!({"width": 32, "height": 24});

    let existing = directory.path().join("existing.png");
    std::fs::write(&existing, b"the user's picture").unwrap();
    assert_eq!(
        bridge.execute(&mut app, presentation(&existing, small.clone()), false),
        Err("export_failed")
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"the user's picture");

    let unknown = directory.path().join("glass.png");
    let mut image = small.clone();
    image["opacity_percent"] = json!({"Glass": 40});
    assert_eq!(
        bridge.execute(&mut app, presentation(&unknown, image), false),
        Err("export_refused")
    );
    assert!(
        app.action_digest().contains("Glass"),
        "{}",
        app.action_digest()
    );
    assert!(!unknown.exists());

    for wrong in [
        json!({"background": "#12345"}),
        json!({"opacity_percent": {MEMBRANE: 101}}),
        json!({"labels": [{"text": " ", "at_mm": [0, 0, 0]}]}),
        json!({"labels": [{"text": "two\nlines", "at_mm": [0, 0, 0]}]}),
    ] {
        let target = directory.path().join("wrong.png");
        assert_eq!(
            bridge.execute(&mut app, presentation(&target, wrong), false),
            Err("invalid_params")
        );
        assert!(!target.exists());
    }
    let mut not_an_image = presentation(&directory.path().join("model.glb"), small);
    if let Request::Export { export_format, .. } = &mut not_an_image {
        "glb".clone_into(export_format);
    }
    assert_eq!(
        bridge.execute(&mut app, not_an_image, false),
        Err("invalid_params")
    );
}
