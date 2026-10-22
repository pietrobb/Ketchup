//! The drawing tools and Measure snap to the ends, middles and lines of a shown underlay,
//! Measure reads the length of one of its lines, and a hidden layer gives nothing.
use crate::*;

/// On "walls" a 200 mm line and a quarter arc of radius 50 from its end; on "axes" a line
/// 100 mm above it, and one across both at x = 60.
const PLAN_DXF: &str = concat!(
    "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n8\nwalls\n10\n40\n20\n20\n11\n240\n21\n20\n",
    "0\nARC\n8\nwalls\n10\n240\n20\n70\n40\n50\n50\n270\n51\n0\n",
    "0\nLINE\n8\naxes\n10\n40\n20\n120\n11\n240\n21\n120\n",
    "0\nLINE\n8\naxes\n10\n60\n20\n0\n11\n60\n21\n150\n",
    "0\nENDSEC\n0\nEOF\n",
);

fn viewport() -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))
}

pub(super) fn imported() -> KetchupApp {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("plan.dxf");
    std::fs::write(&path, PLAN_DXF).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &path);
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    app.dispatch_file_command(AppCommand::ImportDrawingUnderlay);
    assert_eq!(
        app.document.current().underlays().count(),
        1,
        "{}",
        app.action_digest()
    );
    // Built on the thread that read the drawing: there without waiting.
    assert!(app.underlay_snaps_built());
    app
}

/// While the snap index is still being built off the window thread, the underlay gives no
/// snap and says so; once it is there, the same point snaps.
#[test]
fn an_underlay_gives_no_snap_until_its_snap_index_is_built() {
    use crate::app::underlay_snapping::UnderlaySnapSlot;
    use ketchup_model::underlay::UnderlayLines;
    use ketchup_model::underlay::snap::UnderlaySnapIndex;

    let app = imported();
    let snapshot = app.document.current();
    let underlay = snapshot.underlays().next().unwrap();
    let lines = UnderlayLines::decode(
        app.file
            .container_data
            .blobs()
            .get(&underlay.lines_blob())
            .unwrap(),
    )
    .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.render
        .underlay_snaps
        .borrow_mut()
        .insert(underlay.lines_sha256, UnderlaySnapSlot::Building(receiver));

    let end = Vec3::new(240.0, 20.0, 0.0);
    assert_eq!(snap_near(&app, end, Vec2::new(2.0, 1.0)), None);
    let (found, ready) = app.underlay_snaps_near(&snapshot, [240.0, 20.0], 5.0);
    assert!(found.is_empty() && !ready);

    sender.send(UnderlaySnapIndex::new(&lines)).unwrap();
    let snap = snap_near(&app, end, Vec2::new(2.0, 1.0)).unwrap();
    assert_eq!((snap.kind, snap.position_mm), (SnapKind::Endpoint, end));
    assert!(app.underlay_snaps_near(&snapshot, [240.0, 20.0], 5.0).1);
}

fn snap_near(app: &KetchupApp, point: Vec3, offset: Vec2) -> Option<SnapResult> {
    let rect = viewport();
    app.scene_snap_at_screen(app.project(point, rect) + offset, rect, 8.0, None)
}

#[test]
fn drawing_and_measure_snap_to_the_end_middle_and_line_of_a_shown_underlay() {
    let mut app = imported();
    let rect = viewport();
    let end = Vec3::new(240.0, 20.0, 0.0);
    let snap = snap_near(&app, end, Vec2::new(2.0, 1.0)).unwrap();
    assert_eq!((snap.kind, snap.position_mm), (SnapKind::Endpoint, end));
    assert!(matches!(snap.reference.element, ElementId::Underlay { .. }));

    let middle = Vec3::new(140.0, 20.0, 0.0);
    let snap = snap_near(&app, middle, Vec2::new(0.0, 3.0)).unwrap();
    assert_eq!((snap.kind, snap.position_mm), (SnapKind::Midpoint, middle));

    let snap = snap_near(&app, Vec3::new(90.0, 20.0, 0.0), Vec2::new(0.0, 3.0)).unwrap();
    assert_eq!(snap.kind, SnapKind::Edge);
    assert!((snap.position_mm.y - 20.0).abs() < 1e-9 && snap.position_mm.z == 0.0);
    assert!((snap.position_mm.x - 90.0).abs() < 5.0, "{snap:?}");

    // The Line tool's hover and its click both take the endpoint.
    let pointer = app.project(end, rect) + Vec2::new(2.0, 1.0);
    app.active_tool = ActiveTool::Line;
    app.update_viewport_inference(Some(pointer), rect);
    assert_eq!(app.hovered_snap_kind(), Some(SnapKind::Endpoint));
    assert_eq!(app.viewport_point_at_screen(pointer, rect, 0.0), Some(end));

    // Measure picks the same points, and Ctrl over the line reads its whole length.
    app.active_tool = ActiveTool::Measure;
    assert_eq!(
        app.measurement_point_at_screen(pointer, rect, 0.0),
        Some(end)
    );
    let on_line = app.project(Vec3::new(190.0, 20.0, 0.0), rect) + Vec2::new(0.0, 2.0);
    let (points, length) = app.edge_at_screen(on_line, rect, 8.0).unwrap();
    assert_eq!(length, 200.0);
    assert_eq!(
        points,
        [Vec3::new(40.0, 20.0, 0.0), Vec3::new(240.0, 20.0, 0.0)]
    );
    // The quarter arc reads its length along its chords, within 0.1 mm of the arc.
    let crest = Vec3::new(
        240.0 + 50.0 * std::f64::consts::FRAC_PI_4.cos(),
        70.0 - 50.0 * std::f64::consts::FRAC_PI_4.sin(),
        0.0,
    );
    let (_, arc) = app
        .edge_at_screen(app.project(crest, rect), rect, 8.0)
        .unwrap();
    let quarter = std::f64::consts::FRAC_PI_2 * 50.0;
    assert!(arc < quarter && quarter - arc < 0.1, "{arc}");
}

#[test]
fn drawing_and_measure_snap_where_underlay_lines_cross_and_measure_between_snaps() {
    let mut app = imported();
    let rect = viewport();
    let cross = Vec3::new(60.0, 20.0, 0.0);
    let snap = snap_near(&app, cross, Vec2::new(2.0, 1.0)).unwrap();
    assert_eq!(snap.kind, SnapKind::Intersection);
    assert!((snap.position_mm - cross).length() < 1e-9, "{snap:?}");

    let pointer = app.project(cross, rect) + Vec2::new(2.0, 1.0);
    app.active_tool = ActiveTool::Line;
    app.update_viewport_inference(Some(pointer), rect);
    assert_eq!(app.hovered_snap_kind(), Some(SnapKind::Intersection));

    // Measure from the crossing to the wall's end: 180 mm along it.
    app.active_tool = ActiveTool::Measure;
    let from = app.measurement_point_at_screen(pointer, rect, 0.0).unwrap();
    let end = Vec3::new(240.0, 20.0, 0.0);
    let to = app
        .measurement_point_at_screen(app.project(end, rect) + Vec2::new(2.0, 1.0), rect, 0.0)
        .unwrap();
    assert_eq!(to, end);
    app.add_measured_point(from);
    app.add_measured_point(to);
    assert!(((to - from).length() - 180.0).abs() < 1e-9);
    assert!(app.digest.contains("180"), "{}", app.digest);

    // The crossing of a shown and a hidden line is no snap.
    let underlay = app.document.current().underlays().next().unwrap().clone();
    let axes = underlay.layers.iter().find(|layer| layer.name == "axes");
    assert!(app.set_tag_visibility(axes.unwrap().tag, false));
    let snap = snap_near(&app, cross, Vec2::new(2.0, 1.0)).unwrap();
    assert_ne!(snap.kind, SnapKind::Intersection);
    // The underlay itself is read, never changed.
    assert_eq!(
        app.document
            .current()
            .underlays()
            .next()
            .unwrap()
            .lines_sha256,
        underlay.lines_sha256
    );
}

/// After a two-point fit the wall's ends lie on the picked model points: drawing and
/// Measure snap there and no longer where the wall was, and a scaled fit measures longer.
/// Each fit is one Undo step; two picks at one point are refused and change nothing.
#[test]
fn drawing_and_measure_snap_to_the_lines_where_a_two_point_fit_placed_them() {
    use crate::app::underlay::UnderlayEditError;
    use ketchup_model::underlay::placement::UnderlayFitError;

    let mut app = imported();
    let rect = viewport();
    let id = app.document.current().underlays().next().unwrap().id;
    let undo = app.undo_step_count();
    let wall = [[40.0, 20.0], [240.0, 20.0]];
    let target = [[300.0, 100.0], [300.0, 300.0]];
    let fit = app.fit_underlay(id, wall, target, false).unwrap();
    assert!((fit.turn_deg - 90.0).abs() < 1e-9 && fit.length_mismatch_mm == 0.0);
    assert_eq!(app.undo_step_count(), undo + 1);

    let end = Vec3::new(300.0, 300.0, 0.0);
    let snap = snap_near(&app, end, Vec2::new(2.0, 1.0)).unwrap();
    assert_eq!(snap.kind, SnapKind::Endpoint);
    assert!((snap.position_mm - end).length() < 1e-9, "{snap:?}");
    assert_eq!(
        snap_near(&app, Vec3::new(240.0, 20.0, 0.0), Vec2::ZERO),
        None
    );
    let on_wall = app.project(Vec3::new(300.0, 250.0, 0.0), rect) + Vec2::new(2.0, 0.0);
    let (points, length) = app.edge_at_screen(on_wall, rect, 8.0).unwrap();
    assert!((length - 200.0).abs() < 1e-9);
    assert!((points[0] - Vec3::new(300.0, 100.0, 0.0)).length() < 1e-9);

    // Scaled onto a wall twice as long, from where its ends are drawn now.
    let twice = [[300.0, 100.0], [300.0, 500.0]];
    let fit = app.fit_underlay(id, target, twice, true).unwrap();
    assert!((fit.scale - 2.0).abs() < 1e-9);
    let (_, length) = app.edge_at_screen(on_wall, rect, 8.0).unwrap();
    assert!((length - 400.0).abs() < 1e-9, "{length}");

    let digest = app.canonical_digest();
    let refused = app.fit_underlay(id, [[1.0, 1.0], [1.0, 1.0]], twice, false);
    assert!(matches!(
        refused,
        Err(UnderlayEditError::Fit(
            UnderlayFitError::UnderlayPointsCoincide
        ))
    ));
    assert_eq!(
        (app.canonical_digest(), app.undo_step_count()),
        (digest, undo + 2)
    );
}

/// The Fit underlay tool in the window: two clicks on the wall's ends, then two model points,
/// place the wall's ends on those points as one Undo step, and snapping follows. A click off
/// the underlay's lines is named and picks nothing; Esc drops the picks and leaves the tool.
#[test]
fn the_fit_underlay_tool_places_two_clicked_underlay_points_on_two_clicked_model_points() {
    let mut app = imported();
    let rect = viewport();
    let click = |app: &mut KetchupApp, point: Vec3| {
        let pointer = app.project(point, rect) + Vec2::new(2.0, 1.0);
        app.pick_underlay_fit_point(pointer, rect);
    };
    app.dispatch_command(AppCommand::FitUnderlay);
    assert_eq!(app.active_tool, ActiveTool::FitUnderlay);
    assert!(!app.gesture.underlay_fit.scaled);
    let undo = app.undo_step_count();
    let digest = app.canonical_digest();

    click(&mut app, Vec3::new(600.0, 600.0, 0.0));
    assert!(app.gesture.underlay_fit.from.is_none());
    assert_eq!(
        app.digest,
        app.catalog.text("digest-fit-underlay-line-required")
    );

    click(&mut app, Vec3::new(40.0, 20.0, 0.0));
    click(&mut app, Vec3::new(240.0, 20.0, 0.0));
    assert_eq!(app.status_key, "status-fit-underlay-first-model");
    app.cancel_or_deselect(false);
    assert_eq!(app.active_tool, ActiveTool::Select);
    assert!(app.gesture.underlay_fit.from.is_none());
    assert_eq!(
        (app.canonical_digest(), app.undo_step_count()),
        (digest, undo)
    );

    app.dispatch_command(AppCommand::FitUnderlay);
    let targets = [Vec3::new(400.0, 100.0, 0.0), Vec3::new(400.0, 300.0, 0.0)];
    let model: Vec<Vec3> = targets
        .iter()
        .map(|target| {
            let pointer = app.project(*target, rect) + Vec2::new(2.0, 1.0);
            app.measurement_point_at_screen(pointer, rect, 0.0).unwrap()
        })
        .collect();
    click(&mut app, Vec3::new(40.0, 20.0, 0.0));
    click(&mut app, Vec3::new(240.0, 20.0, 0.0));
    click(&mut app, targets[0]);
    click(&mut app, targets[1]);
    assert_eq!(app.undo_step_count(), undo + 1, "{}", app.digest);
    assert!(app.gesture.underlay_fit.from.is_none() && app.gesture.underlay_fit.to.is_empty());

    // The first wall end sits on the first model point; the second on the line to the
    // second model point, 200 mm along it, since the fit does not scale.
    let direction = (model[1] - model[0]) * (1.0 / (model[1] - model[0]).length());
    for (end, expected) in [
        (model[0], model[0]),
        (model[0] + direction * 200.0, model[0] + direction * 200.0),
    ] {
        let snap = snap_near(&app, end, Vec2::new(2.0, 1.0)).unwrap();
        assert_eq!(snap.kind, SnapKind::Endpoint, "{snap:?}");
        assert!((snap.position_mm - expected).length() < 1e-6, "{snap:?}");
    }
    assert_eq!(
        snap_near(&app, Vec3::new(240.0, 20.0, 0.0), Vec2::ZERO),
        None
    );
}

/// Review 2026-10-21 P3: the points picked on the underlay were kept when the document
/// changed under the tool, by Undo or by an MCP fit, so the next two model clicks placed
/// the underlay from where its lines no longer are. The picks are dropped instead.
#[test]
fn the_fit_underlay_tool_drops_its_picks_when_the_document_changes_under_it() {
    let mut app = imported();
    let rect = viewport();
    let id = app.document.current().underlays().next().unwrap().id;
    let click = |app: &mut KetchupApp, point: Vec3| {
        let pointer = app.project(point, rect) + Vec2::new(2.0, 1.0);
        app.pick_underlay_fit_point(pointer, rect);
    };
    // A first fit, so that Undo has something to take back.
    app.fit_underlay(
        id,
        [[40.0, 20.0], [240.0, 20.0]],
        [[40.0, 0.0], [240.0, 0.0]],
        false,
    )
    .unwrap();
    app.dispatch_command(AppCommand::FitUnderlay);
    click(&mut app, Vec3::new(40.0, 0.0, 0.0));
    click(&mut app, Vec3::new(240.0, 0.0, 0.0));
    assert!(app.gesture.underlay_fit.from.is_some());
    assert!(app.undo());
    assert!(app.gesture.underlay_fit.from.is_none(), "kept after Undo");

    // An MCP fit while two underlay points are picked: the next click starts over.
    app.dispatch_command(AppCommand::FitUnderlay);
    click(&mut app, Vec3::new(40.0, 20.0, 0.0));
    click(&mut app, Vec3::new(240.0, 20.0, 0.0));
    app.fit_underlay(
        id,
        [[40.0, 20.0], [240.0, 20.0]],
        [[40.0, 50.0], [240.0, 50.0]],
        false,
    )
    .unwrap();
    let undo = app.undo_step_count();
    click(&mut app, Vec3::new(400.0, 100.0, 0.0));
    assert_eq!(
        app.digest,
        app.catalog.text("digest-fit-underlay-picks-dropped")
    );
    assert_eq!(app.status_key, "status-fit-underlay-first-line");
    click(&mut app, Vec3::new(400.0, 300.0, 0.0));
    assert_eq!(app.undo_step_count(), undo, "{}", app.digest);
}

#[test]
fn a_hidden_underlay_layer_or_underlay_gives_no_snap() {
    let mut app = imported();
    let underlay = app.document.current().underlays().next().unwrap().clone();
    let end = Vec3::new(240.0, 20.0, 0.0);
    let axis = Vec3::new(140.0, 120.0, 0.0);
    let walls = underlay.layers.iter().find(|layer| layer.name == "walls");
    assert!(app.set_tag_visibility(walls.unwrap().tag, false));
    assert_eq!(snap_near(&app, end, Vec2::new(2.0, 1.0)), None);
    assert_eq!(
        snap_near(&app, axis, Vec2::ZERO).map(|snap| snap.kind),
        Some(SnapKind::Midpoint)
    );
    assert!(app.set_tag_visibility(underlay.tag, false));
    assert_eq!(snap_near(&app, axis, Vec2::ZERO), None);
    assert!(app.set_tag_visibility(underlay.tag, true));
    assert!(app.set_tag_visibility(walls.unwrap().tag, true));
    assert!(snap_near(&app, end, Vec2::new(2.0, 1.0)).is_some());
}
