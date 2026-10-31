//! V9d: the whole round trip with the real Viewer, not hand-written comment
//! files. The author exports a scene with notes and a dimension; the customer
//! opens it in the Viewer, sees them, turns the model, comments with an arrow
//! and returns the file; the author opens it and sees the model as the
//! customer did, with the arrow on the same point of the same part.

use super::viewer_settings_tests::install_geometry;
use crate::*;
use egui_kittest::{Harness, kittest::Queryable as _};
use ketchup_manufacturing::viewer_export::{received, view};
use ketchup_model::document::RuleProgramSource;
use ketchup_viewer::app::ViewerApp;
use ketchup_viewer::camera::Camera as ViewerCamera;
use ketchup_viewer::model::Model as ViewerModel;
use ketchup_viewer::text::Language;

const DEFINITION_NOTE: &str = "HPL 0.8 mm on both faces";
const OCCURRENCE_NOTE: &str = "Under the window";
const COMMENT: &str = "Make the board 20 mm thicker";

/// One HPL board, one scene, both notes and a measured dimension.
fn author() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        RuleProgramSource {
            variants: Default::default(),
            file_name: "board.star".into(),
            source: "a=box(\"board\", (600,18,400), material=\"HPL\")\nc=component(\"desk\",[a])"
                .into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .expect("program");
    install_geometry(&mut app);
    app.camera.viewport_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)));
    app.zoom_fit();
    app.save_view("Desk for Jana").expect("scene");
    let board = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .find(|item| !item.instance_path.is_root())
        .expect("board");
    assert!(app.set_viewer_definition_note(board.definition_id, DEFINITION_NOTE));
    assert!(app.set_viewer_occurrence_note(&board.instance_path, OCCURRENCE_NOTE));
    app.add_measured_point(Vec3::new(0.0, 0.0, 0.0));
    app.add_measured_point(Vec3::new(600.0, 0.0, 0.0));
    assert!(app.add_viewer_measured_dimension());
    app
}

fn to_screen(rect: Rect, ndc: [f32; 3]) -> Pos2 {
    Pos2::new(
        rect.left() + (ndc[0] + 1.0) * 0.5 * rect.width(),
        rect.top() + (1.0 - ndc[1]) * 0.5 * rect.height(),
    )
}

fn click(harness: &mut Harness<'_, ViewerApp>, at: Pos2) {
    for pressed in [true, false] {
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        harness.run();
    }
}

fn orbit(harness: &mut Harness<'_, ViewerApp>, from: Pos2) {
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(from),
        egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    harness.run();
    for step in 1..=6 {
        let at = from + Vec2::new(12.0 * step as f32, -5.0 * step as f32);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        harness.run();
    }
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: from + Vec2::new(72.0, -30.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
}

#[test]
fn a_customer_comment_made_in_the_viewer_shows_the_author_the_customer_s_view_and_arrow() {
    // The author exports.
    let author = author();
    let settings = author.stored_viewer_settings().expect("settings");
    let package = author
        .current_model_viewer_export(
            &author.viewer_export_scenes(&settings),
            author.camera.viewport_rect.unwrap(),
        )
        .expect("export")
        .package;
    let mut bytes = Vec::new();
    package.write(&mut bytes).expect("package");
    let model = ViewerModel::read(bytes.as_slice()).expect("the Viewer reads the export");
    assert_eq!(model.manifest().scenes.len(), 1);
    {
        // The scene shows the Viewer exactly what the author saved.
        let rect = author.camera.viewport_rect.unwrap();
        let scene = ViewerCamera::from_scene(&model, &model.manifest().scenes[0].camera).unwrap();
        for point in [[0.0, 0.0, 0.0], [600.0, 18.0, 400.0], [-150.0, 320.0, 15.0]] {
            let viewer = to_screen(rect, scene.project(point, rect.width() / rect.height()));
            let desk = author.project(Vec3::from(point.map(f64::from)), rect);
            assert!(
                viewer.distance(desk) < 0.5,
                "{point:?}: {desk:?} vs {viewer:?}"
            );
        }
    }
    let visible = vec![true; model.manifest().occurrences.len()];
    assert_eq!(
        ketchup_viewer::dimensions::placed(&model, Some(0), &visible, None)
            .iter()
            .map(|placed| placed.text.as_str())
            .collect::<Vec<_>>(),
        ["600 mm"],
        "the scene shows the author's dimension"
    );

    // The customer opens it in the Viewer: the scene, then the board's notes.
    let mut customer = ViewerApp::new(None).with_language(Language::ENGLISH);
    customer.open(bytes.as_slice()).expect("open");
    let mut viewer = Harness::builder()
        .with_size(Vec2::new(900.0, 700.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), customer);
    viewer.run();
    viewer.get_by_label("Desk for Jana");
    viewer.get_by_label("Show dimensions");
    let rect = viewer.get_by_label("Model view").rect();
    let aspect = rect.width() / rect.height();
    let scene = ViewerCamera::from_scene(&model, &model.manifest().scenes[0].camera).unwrap();
    click(
        &mut viewer,
        to_screen(rect, scene.project([300.0, 9.0, 200.0], aspect)),
    );
    viewer.get_by_label(DEFINITION_NOTE);
    viewer.get_by_label(OCCURRENCE_NOTE);

    // Turned to a view of their own, a comment with an arrow from the
    // middle of the view.
    orbit(&mut viewer, rect.center() + Vec2::new(-200.0, 120.0));
    viewer.get_by_label("Add comment").click();
    viewer.run();
    viewer.get_by_label("Comment").focus();
    viewer.run();
    viewer.get_by_label("Comment").type_text(COMMENT);
    viewer.run();
    viewer.get_by_label("Your name (optional)").focus();
    viewer.run();
    viewer
        .get_by_label("Your name (optional)")
        .type_text("Jana");
    viewer.run();
    viewer.get_by_label("Add arrow").click();
    viewer.run();
    click(&mut viewer, rect.center());
    click(&mut viewer, rect.left_top() + Vec2::new(40.0, 40.0));
    viewer.get_by_label("Arrows: 1");
    viewer.get_by_label("Save comment").click();
    viewer.run();
    let mut returned = Vec::new();
    let written = viewer
        .state()
        .save_comments(&mut returned)
        .expect("comments file");
    viewer.state_mut().comments_written(written);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("returned.ketchup-comments");
    std::fs::write(&path, &returned).unwrap();
    the_author_sees_the_customer_s_comment(author, &package, &model, &path, COMMENT);
}

/// The same round trip with the customer's Viewer outside this process: the
/// web Viewer in a browser or the Android Viewer on an emulator. Writes the
/// export to `$KETCHUP_V9D_DIR/model.ketchup-view`, waits up to 20 minutes for
/// the customer's `returned.ketchup-comments` there and checks what the author
/// sees. The web Viewer's half is `viewer/web/tests/customer_round_trip.py`;
/// on Android the export is opened in the Viewer on the emulator and the file
/// saved there with "Save comments for the author" is pulled back with
/// `adb pull` as `returned.ketchup-comments`.
#[test]
#[ignore = "needs the customer's Viewer on another platform: the web Viewer or the Android emulator"]
fn a_customer_comment_made_in_another_viewer_shows_the_author_the_customer_s_view_and_arrow() {
    let directory = std::path::PathBuf::from(
        std::env::var_os("KETCHUP_V9D_DIR").expect("KETCHUP_V9D_DIR names the exchange folder"),
    );
    let text = std::env::var("KETCHUP_V9D_COMMENT").unwrap_or_else(|_| COMMENT.to_owned());
    let author = author();
    let settings = author.stored_viewer_settings().expect("settings");
    let package = author
        .current_model_viewer_export(
            &author.viewer_export_scenes(&settings),
            author.camera.viewport_rect.unwrap(),
        )
        .expect("export")
        .package;
    let mut bytes = Vec::new();
    package.write(&mut bytes).expect("package");
    let model = ViewerModel::read(bytes.as_slice()).expect("the Viewer reads the export");
    let returned = directory.join("returned.ketchup-comments");
    let _ = std::fs::remove_file(&returned);
    std::fs::write(directory.join("model.ketchup-view.part"), &bytes).unwrap();
    std::fs::rename(
        directory.join("model.ketchup-view.part"),
        directory.join("model.ketchup-view"),
    )
    .unwrap();
    eprintln!("export written; waiting for {}", returned.display());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20 * 60);
    while !returned.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "no comments came back"
        );
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    the_author_sees_the_customer_s_comment(author, &package, &model, &returned, &text);
}

/// What the customer returned in `path` is one comment with `text` on the
/// board, in a view of their own, with an arrow onto the board; the author
/// opens it and sees the model as the customer did, the arrow on the same
/// point and the board selected, and the model is not edited.
fn the_author_sees_the_customer_s_comment(
    author: KetchupApp,
    package: &view::Package,
    model: &ViewerModel,
    path: &std::path::Path,
    text: &str,
) {
    let returned = std::fs::read(path).expect("returned comments");
    let comments = view::Comments::read(returned.as_slice(), package).expect("bound comments");
    assert_eq!(comments.comments.len(), 1);
    let comment = comments.comments[0].clone();
    assert_eq!(comment.text, text);
    let board = model
        .manifest()
        .occurrences
        .iter()
        .find(|occurrence| occurrence.name == "board")
        .expect("board")
        .path
        .clone();
    assert_eq!(comment.selected, Some(board.clone()));
    assert_ne!(
        comment.view.camera,
        model.manifest().scenes[0].camera,
        "the customer's own view, not the scene"
    );
    assert_eq!(comment.arrows.len(), 1);
    assert_eq!(comment.arrows[0].tip.occurrence, Some(board.clone()));

    // The author opens the returned file and shows the comment.
    let mut author = author;
    let before = (author.document_revision(), author.undo_step_count());
    assert_eq!(author.import_viewer_comments_from(path).unwrap().added, 1);
    let mut desk = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), author);
    desk.run();
    desk.get_by_label("Jana").click();
    desk.run();
    let app = desk.state();
    assert_eq!(app.edit_context_depth(), 1, "the desk component is opened");
    let rect = app.camera.viewport_rect.unwrap();
    let seen = ViewerCamera::from_scene(model, &comment.view.camera).unwrap();
    let aspect = rect.width() / rect.height();
    // Opening the component adds its breadcrumb, 2 points off the viewport
    // after the view was set; that moves the picture by under a point.
    for point in [[0.0, 0.0, 0.0], [600.0, 18.0, 400.0], [-150.0, 320.0, 15.0]] {
        let customer = to_screen(rect, seen.project(point, aspect));
        let author = app.project(Vec3::from(point.map(f64::from)), rect);
        assert!(
            customer.distance(author) < 1.0,
            "{point:?}: the author sees it at {author:?}, the customer at {customer:?}"
        );
    }
    let snapshot = app.document.current();
    let tip = received::anchor_point(&snapshot, &comment.arrows[0].tip).expect("tip on the board");
    let board = received::model_path(&snapshot, &board).expect("board in the model");
    drop(snapshot);
    let overlay = app.received_comment_overlay(rect).expect("overlay");
    assert_eq!(overlay.arrows.len(), 1);
    let tip_on_screen = to_screen(rect, seen.project(tip.map(|v| v as f32), aspect));
    assert!(
        overlay.arrows[0][1].distance(tip_on_screen) < 1.0,
        "the arrow points where the customer tapped: {:?} vs {tip_on_screen:?}",
        overlay.arrows[0][1]
    );
    assert_eq!(overlay.label.unwrap().1, format!("Jana: {text}"));
    assert!(
        app.selected_instance_paths().contains(&board),
        "the board inside the desk is selected, its component opened"
    );
    assert_eq!(
        before,
        (app.document_revision(), app.undo_step_count()),
        "comments never edit the model"
    );
}
