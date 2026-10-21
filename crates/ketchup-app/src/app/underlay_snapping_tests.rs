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

fn imported() -> KetchupApp {
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
