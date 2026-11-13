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

/// R1112-IMG-500: `{"image":{"height":4}}` with the default corner shading closed the window.
#[test]
fn a_client_saves_an_image_only_a_few_pixels_high() {
    let (mut app, mut bridge, directory) = window();
    for (name, size) in [
        ("four.png", json!({"height": 4})),
        ("one.png", json!({"width": 1, "height": 1})),
    ] {
        let path = directory.path().join(name);
        let answer = bridge
            .execute(&mut app, presentation(&path, size), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        assert_eq!(answer["exported"], true, "{answer}");
        assert!(path.exists());
    }
}

/// R1112-IMG-501: a size past the limit is the client's to change, so it is invalid_params,
/// not export_failed asking to lower an edge smoothing no one can set.
#[test]
fn a_client_image_past_the_pixel_limit_is_invalid_params() {
    let (mut app, mut bridge, directory) = window();
    let path = directory.path().join("huge.png");
    assert_eq!(
        bridge.execute(
            &mut app,
            presentation(&path, json!({"width": 8192, "height": 8192})),
            false
        ),
        Err("invalid_params")
    );
    assert!(!path.exists());
}

/// R1112-IMG-502: a client's image is made on a thread while the window keeps polling, as
/// from the window, and one the client gives up on stops and writes nothing.
#[test]
fn a_clients_image_is_made_off_the_windows_thread_and_a_cancelled_one_writes_nothing() {
    use crate::live_bridge::program_check::ProgramCheckJob;
    use crate::live_bridge::program_dry_run::QueuedReply;
    let (mut app, mut bridge, directory) = window();
    let context = egui::Context::default();
    let small = json!({"width": 64, "height": 48});
    let start = |app: &mut KetchupApp, bridge: &mut LiveBridge, path: &Path| {
        let (reply, answers) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let queued = QueuedReply {
            id: 1,
            reply,
            cancelled: Arc::clone(&cancelled),
        };
        bridge.start_exchange_export(
            app,
            &context,
            queued,
            presentation(path, small.clone()),
            false,
        );
        assert!(
            matches!(
                bridge.program_check_job,
                Some(ProgramCheckJob::Presentation(_))
            ),
            "the image is a job: {}",
            app.action_digest()
        );
        (answers, cancelled)
    };

    let made = directory.path().join("made.png");
    let (answers, _) = start(&mut app, &mut bridge, &made);
    let deadline = Instant::now() + Duration::from_secs(60);
    let answer = loop {
        bridge.poll_program_check_job(&mut app, &context);
        if let Ok(answer) = answers.recv_timeout(Duration::from_millis(2)) {
            break answer;
        }
        assert!(Instant::now() < deadline, "the image was never made");
    };
    assert!(answer.ok, "{answer:?}");
    assert_eq!(pixels(&made).0, 64);

    let given_up = directory.path().join("given-up.png");
    let (answers, cancelled) = start(&mut app, &mut bridge, &given_up);
    cancelled.store(true, Ordering::Release);
    bridge.poll_program_check_job(&mut app, &context);
    let answer = answers.try_recv().expect("answered in the same poll");
    assert_eq!(answer.error.as_deref(), Some("request_cancelled"));
    assert!(bridge.program_check_job.is_none());
    assert!(!given_up.exists());
}

/// Review 2026-11-12 P3: `overwrite` was ignored for an image. With it the window asks the
/// user, and only an agreed replacement is written.
#[test]
fn a_client_image_with_overwrite_replaces_a_file_once_the_user_agrees() {
    let (app, mut bridge, directory) = window();
    let dialogs = crate::dialogs::ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(9);
    let probe = dialogs.clone();
    let mut app = app.with_dialogs(Box::new(dialogs));
    let existing = directory.path().join("existing.png");
    std::fs::write(&existing, b"the user's picture").unwrap();
    let mut request = presentation(&existing, json!({"width": 32, "height": 24}));
    if let Request::Export { overwrite, .. } = &mut request {
        *overwrite = true;
    }
    assert_eq!(
        bridge.execute(&mut app, request.clone(), false),
        Err("export_declined")
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"the user's picture");
    let answer = bridge
        .execute(&mut app, request, false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["exported"], true, "{answer}");
    assert_eq!(pixels(&existing).0, 32);
    assert_eq!(probe.high_risk_prompts().len(), 2);
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

    // R1112-IMG-503: a label far from the model framed it as a speck and said OK.
    let away = directory.path().join("away.png");
    let mut image = small.clone();
    image["labels"] = json!([{"text": "far", "at_mm": [1e12, 0, 0]}]);
    assert_eq!(
        bridge.execute(&mut app, presentation(&away, image), false),
        Err("export_refused")
    );
    assert!(!away.exists());

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
