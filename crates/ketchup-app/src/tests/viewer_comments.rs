//! Customer comments returned from the Viewer, opened by the author.

use super::*;
use ketchup_manufacturing::viewer_export::DesktopCameraContext;
use ketchup_model::document::SavedCamera;
use ketchup_view_format as view;

const VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

fn customer_camera() -> SavedCamera {
    SavedCamera {
        parallel: true,
        yaw_rad: 0.9,
        pitch_rad: 1.1,
        target_z_mm: 30.0,
        zoom: 1.5,
        pan: [40.0, -20.0],
    }
}

/// An app with one exported scene and the package written for it.
fn exported() -> (KetchupApp, view::Package) {
    let mut app = KetchupApp::new();
    let rect = Rect::from_min_size(Pos2::ZERO, VIEWPORT);
    app.camera.viewport_rect = Some(rect);
    let scene = app.save_view("Scene").unwrap();
    install_initial_graph_result(&mut app);
    let package = app
        .current_model_viewer_export(&[scene], rect)
        .unwrap()
        .package;
    (app, package)
}

fn comments_on(package: &view::Package) -> view::Comments {
    let short = f64::from(VIEWPORT.x.min(VIEWPORT.y));
    let saved = customer_camera();
    let context = DesktopCameraContext {
        viewport_points: [f64::from(VIEWPORT.x), f64::from(VIEWPORT.y)],
        orbit_target_xy_mm: [BOX_WIDTH_MM * 0.5, BOX_DEPTH_MM * 0.5],
        points_per_mm: saved.zoom * short / 420.0,
        eye_distance_mm: 2_000.0,
    };
    let part = package.manifest.occurrences[0].path.clone();
    let mut comments = view::Comments::new(package).unwrap();
    comments.comments.push(view::Comment {
        id: 1,
        author: Some("Customer A".into()),
        text: "Please round this corner".into(),
        view: view::CommentView {
            camera: context.camera(&saved).unwrap(),
            hidden: vec![],
            style: view::DisplayStyle::Wireframe,
            section: None,
        },
        selected: Some(part.clone()),
        arrows: vec![view::Arrow {
            tip: view::Anchor {
                occurrence: Some(part),
                point_mm: [0.0, 0.0, 0.0],
            },
            tail: view::Anchor {
                occurrence: None,
                point_mm: [-300.0, -300.0, 400.0],
            },
        }],
    });
    comments
}

fn package_part(package: &view::Package) -> InstancePath {
    InstancePath::root(OccurrenceId(
        package.manifest.occurrences[0].path.root_occurrence_id,
    ))
}

fn write_comments(path: &Path, comments: &view::Comments, package: &view::Package) {
    comments
        .write(std::fs::File::create(path).unwrap(), package)
        .unwrap();
}

fn screen_points(app: &KetchupApp, rect: Rect) -> Vec<Pos2> {
    [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(BOX_WIDTH_MM, 40.0, 90.0),
        Vec3::new(-150.0, 320.0, 15.0),
    ]
    .into_iter()
    .map(|point| app.project(point, rect))
    .collect()
}

#[test]
fn author_opens_customer_comment_with_its_camera_part_and_arrow_without_editing() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, package) = exported();
    let comments = comments_on(&package);
    let path = directory.path().join("returned.ketchup-comments");
    write_comments(&path, &comments, &package);
    let before = (app.document_revision(), app.undo_step_count());

    let counts = app.import_viewer_comments_from(&path).unwrap();
    assert_eq!(counts.added, 1);
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let title = harness
        .state()
        .catalog
        .text("viewer-received-comments-title");
    harness.get_by_label(&title);
    harness.get_by_label("Please round this corner");
    harness.get_by_label("Customer A").click();
    harness.run();

    let app = harness.state();
    assert_eq!(app.viewer_prep.received.active, Some(0));
    // Where the customer saw the model, framed by the short side of this
    // viewport as the Viewer frames it on any screen.
    let rect = app.camera.viewport_rect.unwrap();
    let ratio = rect.width().min(rect.height()) / VIEWPORT.x.min(VIEWPORT.y);
    let saved = customer_camera();
    let (mut reference, _) = exported();
    reference.camera.projection_mode = ProjectionMode::Parallel;
    reference.camera.yaw = saved.yaw_rad as f32;
    reference.camera.pitch = saved.pitch_rad as f32;
    reference.camera.target_z = saved.target_z_mm;
    reference.camera.zoom = saved.zoom as f32;
    reference.camera.pan = Vec2::new(saved.pan[0] as f32, saved.pan[1] as f32) * ratio;
    let expected = screen_points(&reference, rect);
    let actual = screen_points(app, rect);
    for (actual, expected) in actual.iter().zip(&expected) {
        assert!(
            actual.distance(*expected) < 0.05,
            "{actual:?} != {expected:?}"
        );
    }
    // The arrow ends at the part's local origin, labelled with the comment.
    let snapshot = app.document.current();
    let part_origin = snapshot
        .resolve_instance_path(&package_part(&package))
        .unwrap()
        .world_transform
        .transform_point([0.0; 3]);
    drop(snapshot);
    let overlay = app.received_comment_overlay(rect).unwrap();
    assert_eq!(overlay.arrows.len(), 1);
    assert!(overlay.arrows[0][1].distance(app.project(Vec3::from(part_origin), rect)) < 1e-3);
    let (at, text) = overlay.label.unwrap();
    assert!(rect.contains(at) && overlay.arrows[0].contains(&at));
    assert_eq!(text, "Customer A: Please round this corner");
    assert!(app.view.contains(ViewFlag::Wireframe));
    assert!(
        app.selected_instance_paths()
            .contains(&package_part(&package))
    );
    assert_eq!(before, (app.document_revision(), app.undo_step_count()));
}

#[test]
fn reimport_never_duplicates_and_other_projects_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, package) = exported();
    let mut comments = comments_on(&package);
    let file = directory.path().join("returned.ketchup-comments");
    write_comments(&file, &comments, &package);
    let commented = directory.path().join("commented.ketchup-view");
    package
        .write_with_comments(std::fs::File::create(&commented).unwrap(), &comments)
        .unwrap();
    let plain = directory.path().join("plain.ketchup-view");
    package
        .write(std::fs::File::create(&plain).unwrap())
        .unwrap();
    let before = (app.document_revision(), app.undo_step_count());

    assert_eq!(app.import_viewer_comments_from(&file).unwrap().added, 1);
    let again = app.import_viewer_comments_from(&commented).unwrap();
    assert_eq!((again.added, again.updated, again.unchanged), (0, 0, 1));
    assert!(app.import_viewer_comments_from(&plain).is_err());

    // The customer edited the comment and sent it again: replaced, not added.
    comments.comments[0].text = "Round both corners".into();
    write_comments(&file, &comments, &package);
    let edited = app.import_viewer_comments_from(&file).unwrap();
    assert_eq!((edited.added, edited.updated), (0, 1));
    assert_eq!(app.received_viewer_comments().len(), 1);
    assert_eq!(
        app.received_viewer_comments()[0].comment.text,
        "Round both corners"
    );

    let mut foreign = comments.clone();
    foreign.export.source.document_id ^= 1;
    let foreign_file = directory.path().join("foreign.ketchup-comments");
    std::fs::write(&foreign_file, serde_json::to_vec(&foreign).unwrap()).unwrap();
    let error = app.import_viewer_comments_from(&foreign_file).unwrap_err();
    assert_eq!(error.code(), "viewer.comments.project");
    assert!(app.digest.contains(error.reason_text()));
    assert_eq!(app.received_viewer_comments().len(), 1);
    assert_eq!(before, (app.document_revision(), app.undo_step_count()));
}

#[test]
fn deleted_commented_part_is_marked_and_never_selected() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, package) = exported();
    let path = directory.path().join("returned.ketchup-comments");
    write_comments(&path, &comments_on(&package), &package);
    app.import_viewer_comments_from(&path).unwrap();
    let snapshot = app.document.current();
    let status = app.received_comment_status(&snapshot, &app.received_viewer_comments()[0]);
    assert_eq!((status.changed, status.missing), (false, 0));
    drop(snapshot);

    let part = OccurrenceId(package.manifest.occurrences[0].path.root_occurrence_id);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence { id: part },
        ]))
        .unwrap();
    let snapshot = app.document.current();
    let status = app.received_comment_status(&snapshot, &app.received_viewer_comments()[0]);
    assert_eq!((status.changed, status.missing), (true, 1));
    drop(snapshot);
    let revision = app.document_revision();
    assert!(app.show_received_viewer_comment(0));
    assert!(app.selected_instance_paths().is_empty());
    let rect = app.camera.viewport_rect.unwrap();
    let overlay = app.received_comment_overlay(rect).unwrap();
    assert!(overlay.arrows.is_empty(), "no arrow to a deleted part");
    assert_eq!(app.document_revision(), revision);
}

#[test]
fn moved_part_is_flagged_changed_and_arrow_follows_the_same_occurrence() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, package) = exported();
    let path = directory.path().join("returned.ketchup-comments");
    write_comments(&path, &comments_on(&package), &package);
    app.import_viewer_comments_from(&path).unwrap();

    let part = package_part(&package);
    let moved = Transform::from_translation(250.0, 0.0, 0.0).unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: part.root_occurrence(),
                transform: moved,
            },
        ]))
        .unwrap();
    let snapshot = app.document.current();
    let status = app.received_comment_status(&snapshot, &app.received_viewer_comments()[0]);
    assert_eq!((status.changed, status.missing), (true, 0));
    drop(snapshot);

    let revision = app.document_revision();
    assert!(app.show_received_viewer_comment(0));
    let selected = app.selected_instance_paths();
    assert_eq!(selected.len(), 1);
    assert!(selected.contains(&part), "same occurrence, not a lookalike");
    let rect = app.camera.viewport_rect.unwrap();
    let overlay = app.received_comment_overlay(rect).unwrap();
    let tip = app.project(Vec3::new(250.0, 0.0, 0.0), rect);
    assert_eq!(overlay.arrows.len(), 1);
    assert!(
        (overlay.arrows[0][1] - tip).length() < 0.5,
        "arrow points at the part where it is now"
    );
    assert_eq!(app.document_revision(), revision, "comments never edit");
}
