//! A drawing imported 300 m from the origin, as profiles or as an underlay, into a document
//! with nothing else: the view frames it after the import, and every zoom fit and standard
//! view frames it again. A hidden layer of an underlay is left out of the frame.

use super::*;

/// The drawing: a 40 × 30 m rectangle of lines on layer `walls` at x 300–340 m, and on
/// layer `far` one 10 m line at x −500 m, y −500 m. Millimetres ($INSUNITS 4).
fn drawing() -> String {
    let line = |layer: &str, [x0, y0, x1, y1]: [i32; 4]| {
        format!("0\nLINE\n8\n{layer}\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n")
    };
    let layer = |name: &str| format!("0\nLAYER\n2\n{name}\n70\n0\n62\n7\n6\nCONTINUOUS\n");
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n{}{}0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{}{}{}{}{}0\nENDSEC\n0\nEOF\n",
        layer("walls"),
        layer("far"),
        line("walls", [300_000, 100_000, 340_000, 100_000]),
        line("walls", [340_000, 100_000, 340_000, 130_000]),
        line("walls", [340_000, 130_000, 300_000, 130_000]),
        line("walls", [300_000, 130_000, 300_000, 100_000]),
        line("far", [-500_000, -500_000, -490_000, -500_000]),
    )
}

const WALLS: [Vec3; 2] = [
    Vec3 {
        x: 300_000.0,
        y: 100_000.0,
        z: 0.0,
    },
    Vec3 {
        x: 340_000.0,
        y: 130_000.0,
        z: 0.0,
    },
];

/// Whether `point` lies in the camera's view frustum over `rect`: in front of the eye and
/// inside the clip square.
fn in_frustum(app: &KetchupApp, rect: Rect, point: Vec3) -> bool {
    // Column-major: row `r` of the clip point is the sum of `m[c * 4 + r] * point[c]`.
    let m = app.world_to_clip_f64(rect);
    let [x, y, w] = [0, 1, 3]
        .map(|row| m[row] * point.x + m[4 + row] * point.y + m[8 + row] * point.z + m[12 + row]);
    w > 0.0 && x.abs() <= w && y.abs() <= w
}

fn assert_frames(app: &KetchupApp, [low, high]: [Vec3; 2], case: &str) {
    let rect = app.camera.viewport_rect.expect("a laid-out viewport");
    for corner in 0..8 {
        let pick = |bit: u32, low: f64, high: f64| if corner & bit == 0 { low } else { high };
        let point = Vec3::new(
            pick(1, low.x, high.x),
            pick(2, low.y, high.y),
            pick(4, low.z, high.z),
        );
        assert!(
            in_frustum(app, rect, point),
            "{case}: {point:?} is outside the view of {rect:?}"
        );
    }
}

/// Moves the camera away from the drawing, then frames again as the window and a client
/// do: zoom fit in both projections, and each standard view of a client.
fn assert_every_view_frames(app: &mut KetchupApp, bounds: [Vec3; 2]) {
    for projection in [ProjectionMode::Parallel, ProjectionMode::Perspective] {
        app.camera.projection_mode = projection;
        app.camera.pan = Vec2::new(-5_000.0, 3_000.0);
        app.camera.zoom = 40.0;
        app.dispatch_command(AppCommand::ZoomFit);
        app.refresh_camera_distance();
        assert!(
            !app.camera.zoom_fit_pending,
            "{projection:?}: nothing framed"
        );
        assert_frames(app, bounds, &format!("zoom fit, {projection:?}"));
    }
    for view in [
        AppCommand::ViewTop,
        AppCommand::ViewIso,
        AppCommand::ViewFront,
    ] {
        app.camera.projection_mode = ProjectionMode::Parallel;
        app.dispatch_command(view);
        app.dispatch_command(AppCommand::ZoomFit);
        app.refresh_camera_distance();
        assert_frames(app, bounds, &format!("{view:?}"));
    }
}

fn empty_app_with_drawing() -> (KetchupApp, tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("house.dxf");
    std::fs::write(&path, drawing()).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.document = DocumentStore::new();
    (app, directory, path)
}

/// One frame of the window, which lays out the viewport and applies a pending fit.
fn frame(app: &mut KetchupApp) {
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| app.ui(context));
    app.refresh_camera_distance();
}

#[test]
fn an_underlay_far_from_the_origin_is_framed_after_import_and_by_every_view() {
    let (mut app, _directory, _) = empty_app_with_drawing();
    // Without a frame the drawing is read here, and framed on the first frame.
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    let snapshot = app.document.current();
    let underlay = snapshot
        .underlays()
        .next()
        .unwrap_or_else(|| panic!("no underlay: {}", app.action_digest()))
        .clone();
    assert_eq!(snapshot.occurrences().count(), 0, "only the underlay");
    frame(&mut app);
    assert_frames(&app, WALLS, "after the import");
    assert_every_view_frames(&mut app, WALLS);

    // The far layer is in the frame while shown and out of it once hidden.
    let far = Vec3::new(-500_000.0, -500_000.0, 0.0);
    let rect = app.camera.viewport_rect.unwrap();
    let far_tag = underlay.layers.iter().find(|layer| layer.name == "far");
    assert!(app.set_tag_visibility(far_tag.unwrap().tag, false));
    app.dispatch_command(AppCommand::ZoomFit);
    app.refresh_camera_distance();
    assert_frames(&app, WALLS, "far layer hidden");
    assert!(!in_frustum(&app, rect, far), "a hidden layer is not framed");
    assert!(app.set_tag_visibility(far_tag.unwrap().tag, true));
    app.dispatch_command(AppCommand::ZoomFit);
    assert_frames(&app, [far, WALLS[1]], "far layer shown");

    // The whole underlay hidden leaves nothing to frame.
    assert!(app.set_tag_visibility(underlay.tag, false));
    assert!(app.active_frame_bounds().is_empty());
}

#[test]
fn profiles_far_from_the_origin_are_framed_after_import_and_by_every_view() {
    let (mut app, _directory, path) = empty_app_with_drawing();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
    app.camera.viewport_rect = Some(rect);
    app.import_file(ImportFormat::Dxf, &path, None)
        .unwrap_or_else(|error| panic!("{error}: {}", app.action_digest()));
    assert!(app.document.current().occurrences().count() > 0);
    // A laid-out viewport is framed at once.
    assert!(!app.camera.zoom_fit_pending);
    assert_frames(&app, WALLS, "after the import");
    assert_every_view_frames(&mut app, WALLS);
}
