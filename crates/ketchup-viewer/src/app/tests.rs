use super::*;
use crate::model::tests::package;
use egui_kittest::kittest::Queryable as _;
use ketchup_view_format::{Camera as SceneCamera, Projection, Scene};

pub(crate) fn scene_package() -> ketchup_view_format::Package {
    let mut package = package(0.1);
    let camera = SceneCamera {
        eye_mm: [98.0, 210.0, 500.0],
        target_mm: [98.0, 210.0, 309.0],
        up: [0.0, 1.0, 0.0],
        projection: Projection::Orthographic {
            short_span_mm: 30.0,
        },
    };
    package.manifest.scenes = vec![
        Scene {
            id: 1,
            name: "First scene".into(),
            camera: camera.clone(),
            hidden: vec![package.manifest.occurrences[1].path.clone()],
            style: DisplayStyle::ShadedEdges,
            section: None,
            visible_dimensions: vec![],
        },
        Scene {
            id: 2,
            name: "Second scene".into(),
            camera: SceneCamera {
                target_mm: [508.0, 210.0, 310.0],
                eye_mm: [508.0, 210.0, 500.0],
                ..camera
            },
            hidden: vec![package.manifest.occurrences[0].path.clone()],
            style: DisplayStyle::Wireframe,
            section: Some(Section {
                normal: [1.0, 0.0, 0.0],
                offset_mm: 510.0,
            }),
            visible_dimensions: vec![],
        },
    ];
    package.manifest.start_scene = Some(1);
    package
}

#[test]
fn native_viewport_shows_the_rendered_model_at_its_projected_position() {
    let state = ketchup_test_gpu::render_state();
    eprintln!("Viewer UI adapter: {:?}", state.adapter.get_info());
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(state.clone());
    let mut app = ViewerApp::new(Some(state.clone())).with_language(Language::ENGLISH);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).unwrap();
    app.open(bytes.as_slice()).unwrap();
    app.guides = false;
    app.selected = Some(0);
    let mut ui = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .renderer(renderer)
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    ui.run();
    let image = ui.render().unwrap();
    let rect = ui.get_by_label("Model view").rect();
    let projected = ui
        .state()
        .camera
        .as_ref()
        .unwrap()
        .project([88.0, 208.0, 314.0], rect.width() / rect.height());
    let (target, _) = ui.state().target.as_ref().unwrap();
    let raw = target.read_rgba(&state.device, &state.queue).unwrap();
    let x = ((projected[0] + 1.0) * 0.5 * target.size[0] as f32) as u32;
    let y = ((1.0 - projected[1]) * 0.5 * target.size[1] as f32) as u32;
    let index = ((y * target.size[0] + x) * 4) as usize;
    let offscreen = &raw[index..index + 4];
    let shown = image
        .get_pixel(rect.left() as u32 + x, rect.top() as u32 + y)
        .0;
    eprintln!("Projected triangle raw={offscreen:?}, UI={shown:?}, xy={x},{y}");
    assert!(
        shown[0] >= 230 && shown[1] >= 185 && shown[2] >= 65,
        "linear shaded orange must be encoded as sRGB, not displayed too dark: {shown:?}"
    );
    assert!(
        offscreen[0] > offscreen[2].saturating_add(50),
        "projected triangle is orange on GPU"
    );
    assert!(
        shown[0] > shown[2].saturating_add(50),
        "the same triangle must be visible in the UI"
    );
    let background = &raw[..4];
    let painted = raw
        .chunks_exact(4)
        .filter(|pixel| *pixel != background)
        .count();
    assert!(
        painted > 100 && painted < (target.size[0] * target.size[1] / 3) as usize,
        "the framed triangle occupies a bounded part of the viewport, got {painted} pixels"
    );
}

#[test]
fn scene_dimensions_are_drawn_where_they_project_and_switch_off() {
    use ketchup_view_format::{Anchor, Dimension, LengthUnit};
    let mut package = scene_package();
    let world = |point_mm| Anchor {
        occurrence: None,
        point_mm,
    };
    let on_hidden = |point_mm| Anchor {
        occurrence: Some(package.manifest.occurrences[1].path.clone()),
        point_mm,
    };
    package.manifest.dimensions = vec![
        Dimension {
            id: 1,
            from: on_hidden([0.0; 3]),
            to: on_hidden([10.0, 0.0, 0.0]),
            label_offset_mm: [0.0, -4.0, 0.0],
            value_mm: 10.0,
            display_unit: LengthUnit::Millimetre,
        },
        Dimension {
            id: 2,
            from: world([85.0, 203.0, 320.0]),
            to: world([110.0, 203.0, 320.0]),
            label_offset_mm: [0.0, -4.0, 0.0],
            value_mm: 25.0,
            display_unit: LengthUnit::Millimetre,
        },
    ];
    package.manifest.scenes[0].visible_dimensions = vec![1, 2];
    let mut bytes = Vec::new();
    package.write(&mut bytes).unwrap();
    let state = ketchup_test_gpu::render_state();
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(state.clone());
    let mut app = ViewerApp::new(Some(state)).with_language(Language::ENGLISH);
    app.open(bytes.as_slice()).unwrap();
    app.guides = false;
    let mut ui = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .renderer(renderer)
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    ui.run();
    let shown = ui.render().unwrap();
    let rect = ui.get_by_label("Model view").rect();
    let model = ui.state().model.as_ref().unwrap();
    let placed = dimensions::placed(model, ui.state().scene, &ui.state().visible, None);
    assert_eq!(
        placed.iter().map(|d| d.id).collect::<Vec<_>>(),
        [2],
        "a dimension on a component the scene hides is not shown"
    );
    let camera = ui.state().camera.as_ref().unwrap();
    let aspect = rect.width() / rect.height();
    let corners = [[85.0, 203.0, 320.0], [110.0, 199.0, 320.0]].map(|point| {
        let p = camera.project(point, aspect);
        egui::pos2(
            rect.left() + (p[0] + 1.0) * 0.5 * rect.width(),
            rect.top() + (1.0 - p[1]) * 0.5 * rect.height(),
        )
    });
    let drawn = egui::Rect::from_two_pos(corners[0], corners[1]).expand(30.0);
    ui.get_by_label("Show dimensions").click();
    ui.run();
    assert!(!ui.state().dimensions);
    let hidden = ui.render().unwrap();
    let mut changed = 0;
    for (x, y, pixel) in shown.enumerate_pixels() {
        let point = egui::pos2(x as f32, y as f32);
        // The toolbar's own checkbox changes too; compare the 3D view only.
        if rect.contains(point) && pixel != hidden.get_pixel(x, y) {
            changed += 1;
            assert!(
                drawn.contains(point),
                "only the dimension's own lines and label change, not {point:?}"
            );
        }
    }
    assert!(
        changed > 200,
        "the dimension and its label are drawn: {changed}"
    );
}

#[test]
fn package_open_selection_and_scenes_preserve_source_data_and_failed_open_state() {
    let package = scene_package();
    let mut bytes = Vec::new();
    package.write(&mut bytes).expect("fixture");
    let mut app = ViewerApp::new(None).with_language(Language::ENGLISH);
    app.open(bytes.as_slice()).expect("package open");
    assert_eq!(app.visible, [true, false]);
    assert_eq!(
        app.model.as_ref().expect("model").manifest(),
        &package.manifest
    );
    assert!(app.open(&b"bad data"[..]).is_err());
    assert_eq!(app.scene, Some(0));
    let model = app.model.as_ref().expect("model");
    let camera = app.camera.as_ref().expect("camera");
    let point = camera.project([88.0, 208.0, 312.0], 1.0);
    assert_eq!(
        camera.pick(model, [point[0], point[1]], 1.0, &app.visible, None),
        Some(0)
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();
    let rect = harness.get_by_label("Model view").rect();
    let projected = harness
        .state()
        .camera
        .as_ref()
        .expect("camera")
        .project([88.0, 208.0, 314.0], rect.width() / rect.height());
    let point = egui::pos2(
        rect.left() + (projected[0] + 1.0) * rect.width() * 0.5,
        rect.top() + (1.0 - projected[1]) * rect.height() * 0.5,
    );
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(point));
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    assert_eq!(harness.state().selected, Some(0));
    harness.get_by_label("Source path: occurrence:10");
    harness.get_by_label("Material: Authored material");
    harness.get_by_label("Definition note");
    harness.get_by_label("Size: 10 × 10 × 10 mm");
    harness.get_by_label("Second scene").click();
    harness.run();
    assert_eq!(harness.state().scene, Some(1));
    assert_eq!(harness.state().visible, [false, true]);
    assert!(matches!(harness.state().style, DisplayStyle::Wireframe));
    assert_eq!(
        harness.state().section.as_ref().expect("section").offset_mm,
        510.0
    );
    assert_eq!(
        harness.state().camera.as_ref().expect("camera").center,
        [508.0, 210.0, 310.0]
    );
    harness.set_size(egui::vec2(390.0, 700.0));
    harness.run();
    // Phone layout: one toolbar row, the views behind a menu, the model keeps the screen.
    assert!(harness.query_by_label("Iso").is_none());
    let view_height = harness.get_by_label("Model view").rect().height();
    assert!(view_height > 0.5 * 700.0, "model view {view_height} of 700");
    harness.get_by_label("Views").click();
    harness.run();
    harness.get_by_label("Iso").click();
    harness.run();
    let scale = harness.state().camera.as_ref().expect("camera").scale;
    let yaw = harness.state().camera.as_ref().expect("camera").yaw;
    let touch = |id, phase, x| egui::Event::Touch {
        device_id: egui::TouchDeviceId(0),
        id: egui::TouchId(id),
        phase,
        pos: egui::pos2(x, 250.0),
        force: None,
    };
    harness.input_mut().events.extend([
        touch(1, egui::TouchPhase::Start, 150.0),
        touch(2, egui::TouchPhase::Start, 250.0),
    ]);
    harness.run();
    harness.input_mut().events.extend([
        touch(1, egui::TouchPhase::Move, 100.0),
        touch(2, egui::TouchPhase::Move, 300.0),
    ]);
    harness.run();
    let camera = harness.state().camera.as_ref().expect("camera");
    assert!(
        camera.scale < scale * 0.75,
        "pinch must zoom mobile viewport"
    );
    assert_eq!(camera.yaw, yaw, "pinch must not also orbit");
}

#[test]
fn hide_isolate_views_and_pan_change_only_the_view_in_slovak() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app =
        ViewerApp::new(None).with_language(Language::from_tag("sk").expect("Slovak catalog"));
    app.open(bytes.as_slice()).expect("open");
    let manifest = app.model.as_ref().expect("model").manifest().clone();
    app.selected = Some(0);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();
    harness.get_by_label("Rozmer: 10 × 10 × 10 mm");
    harness.get_by_label("Definition note");
    harness.get_by_label("Izolovať").click();
    harness.run();
    assert_eq!(harness.state().visible, [true, false]);
    harness.get_by_label("Skryť").click();
    harness.run();
    assert_eq!(harness.state().visible, [false, false]);
    assert_eq!(harness.state().selected, None);
    harness.get_by_label("Obnoviť scénu").click();
    harness.run();
    assert_eq!(harness.state().visible, [true, false], "scene visibility");
    harness.get_by_label("Zhora").click();
    harness.run();
    let camera = harness.state().camera.clone().expect("camera");
    assert!((camera.pitch - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    let top = camera.project(camera.center, 1.0);
    assert!(
        top[0].abs() < 1e-4 && top[1].abs() < 1e-4,
        "framed on model"
    );
    let mut panned = camera.clone();
    panned.pan([100.0, 0.0], 500.0);
    let moved = panned.project(camera.center, 1.0);
    assert!(
        (moved[0] - 0.4).abs() < 1e-3 && moved[1].abs() < 1e-4,
        "a 100 px drag on a 500 px view moves the model 1:1 to the right"
    );
    assert_eq!(
        harness.state().model.as_ref().expect("model").manifest(),
        &manifest,
        "the package is never edited"
    );
}

#[test]
fn mouse_navigation_follows_sketchup_and_guides_toggle() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = ViewerApp::new(None).with_language(Language::from_tag("en").expect("en"));
    app.open(bytes.as_slice()).expect("open");
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();
    let rect = harness.get_by_label("Model view").rect();
    let start = rect.center();
    let drag =
        |harness: &mut egui_kittest::Harness<'_, ViewerApp>, button, modifiers: egui::Modifiers| {
            let end = start + egui::vec2(60.0, 0.0);
            harness.input_mut().modifiers = modifiers;
            harness.input_mut().events.extend([
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button,
                    pressed: true,
                    modifiers,
                },
            ]);
            harness.run();
            for step in 1..=6 {
                harness.input_mut().events.push(egui::Event::PointerMoved(
                    start + egui::vec2(10.0 * step as f32, 0.0),
                ));
                harness.run();
            }
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: end,
                button,
                pressed: false,
                modifiers,
            });
            harness.run();
            harness.input_mut().modifiers = egui::Modifiers::NONE;
        };
    for button in [egui::PointerButton::Primary, egui::PointerButton::Middle] {
        let before = harness.state().camera.clone().expect("camera");
        drag(&mut harness, button, egui::Modifiers::NONE);
        let after = harness.state().camera.clone().expect("camera");
        assert!(
            after.yaw < before.yaw,
            "{button:?} drag right orbits the model right"
        );
        assert_eq!(after.center, before.center, "{button:?} orbit does not pan");
    }
    for (button, modifiers) in [
        (egui::PointerButton::Middle, egui::Modifiers::SHIFT),
        (egui::PointerButton::Primary, egui::Modifiers::SHIFT),
        (egui::PointerButton::Secondary, egui::Modifiers::NONE),
    ] {
        let before = harness.state().camera.clone().expect("camera");
        drag(&mut harness, button, modifiers);
        let after = harness.state().camera.clone().expect("camera");
        assert_eq!(
            after.yaw, before.yaw,
            "{button:?}+{modifiers:?} does not orbit"
        );
        assert_ne!(after.center, before.center, "{button:?}+{modifiers:?} pans");
    }
    let camera = harness.state().camera.clone().expect("camera");
    let pointer = rect.center() + egui::vec2(150.0, -80.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pointer));
    harness.run();
    harness.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 120.0),
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    harness.run();
    let zoomed = harness.state().camera.clone().expect("camera");
    assert!(zoomed.scale < camera.scale, "wheel up zooms in");
    assert_ne!(
        zoomed.center, camera.center,
        "towards the pointer, not the center"
    );
    assert!(harness.state().guides, "axes and grid on by default");
    harness.get_by_label("Axes and grid").click();
    harness.run();
    assert!(!harness.state().guides);
    let fitted = Camera::fit(harness.state().model.as_ref().expect("model"));
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(start));
    for pressed in [true, false, true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.step();
    let actual = harness.state().camera.as_ref().expect("camera");
    assert_eq!(
        actual.center, fitted.center,
        "double click centers the whole model"
    );
    assert_eq!(
        actual.scale, fitted.scale,
        "double click fits the whole model"
    );
}

/// Click the 3D view at a world point, or at a screen offset from its corner.
fn click_view(harness: &mut egui_kittest::Harness<'_, ViewerApp>, at: egui::Pos2) {
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

/// The same view up to the f32 precision the 3D camera works in.
fn assert_same_view(a: &SceneCamera, b: &SceneCamera) {
    let close =
        |x: [f64; 3], y: [f64; 3], tolerance: f64| (0..3).all(|i| (x[i] - y[i]).abs() < tolerance);
    assert!(
        close(a.eye_mm, b.eye_mm, 1e-3)
            && close(a.target_mm, b.target_mm, 1e-3)
            && close(a.up, b.up, 1e-6)
            && a.projection == b.projection,
        "{a:?} != {b:?}"
    );
}

fn on_screen(harness: &egui_kittest::Harness<'_, ViewerApp>, point: [f32; 3]) -> egui::Pos2 {
    let rect = harness.get_by_label("Model view").rect();
    let p = harness
        .state()
        .camera
        .as_ref()
        .expect("camera")
        .project(point, rect.width() / rect.height());
    egui::pos2(
        rect.left() + (p[0] + 1.0) * 0.5 * rect.width(),
        rect.top() + (1.0 - p[1]) * 0.5 * rect.height(),
    )
}

#[test]
fn customer_comments_restore_their_view_keep_arrows_on_parts_and_travel_offline() {
    let original = scene_package();
    let mut bytes = Vec::new();
    original.write(&mut bytes).expect("fixture");
    let mut app = ViewerApp::new(None).with_language(Language::ENGLISH);
    app.open(bytes.as_slice()).expect("open");
    app.selected = Some(0);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();
    harness.get_by_label("No comments yet");
    harness.get_by_label("Add comment").click();
    harness.run();
    let written_camera = harness.state().camera.clone().expect("camera").to_scene();
    harness.get_by_label("Comment").focus();
    harness.run();
    harness
        .get_by_label("Comment")
        .type_text("Raise the shelf by 2 cm — ďakujem");
    harness.run();
    harness.get_by_label("Your name (optional)").focus();
    harness.run();
    harness
        .get_by_label("Your name (optional)")
        .type_text("Jana");
    harness.run();

    // An arrow: tap the part it points at, then where it starts.
    harness.get_by_label("Add arrow").click();
    harness.run();
    harness.get_by_label("Tap the point the arrow shows");
    let tip = on_screen(&harness, [88.0, 208.0, 314.0]);
    click_view(&mut harness, tip);
    harness.get_by_label("Tap where the arrow starts");
    let corner = harness.get_by_label("Model view").rect().left_top() + egui::vec2(40.0, 40.0);
    click_view(&mut harness, corner);
    assert_eq!(
        harness.state().selected,
        Some(0),
        "placing an arrow keeps the selection"
    );
    harness.get_by_label("Arrows: 1");
    harness.get_by_label("Save comment").click();
    harness.run();

    let review = harness.state().review.as_ref().expect("review");
    assert!(review.unsaved() && review.draft.is_none());
    let comment = review.comments.comments[0].clone();
    let id = comment.id;
    assert_eq!(comment.text, "Raise the shelf by 2 cm — ďakujem");
    assert_eq!(comment.author.as_deref(), Some("Jana"));
    assert_eq!(comment.view.camera, written_camera);
    let path0 = original.manifest.occurrences[0].path.clone();
    assert_eq!(comment.selected, Some(path0.clone()));
    assert_eq!(
        comment.view.hidden,
        [original.manifest.occurrences[1].path.clone()]
    );
    let arrow = &comment.arrows[0];
    assert_eq!(
        arrow.tip.occurrence,
        Some(path0.clone()),
        "the tip holds on the part"
    );
    assert_eq!(
        arrow.tail.occurrence,
        Some(path0),
        "a free tail follows the same part"
    );
    let model = harness.state().model.as_ref().expect("model");
    let visible = harness.state().visible.clone();
    let tip_world = dimensions::world_point(model, &arrow.tip, &visible).expect("tip");
    assert!(
        (tip_world[0] - 88.0).abs() < 1.5 && (tip_world[1] - 208.0).abs() < 1.5,
        "the arrow points where it was tapped: {tip_world:?}"
    );

    // Move away; showing the comment restores the view it was written in.
    harness.get_by_label("Top").click();
    harness.run();
    harness.get_by_label("Isolate").click();
    harness.run();
    harness.get_by_label("Second scene").click();
    harness.run();
    assert_ne!(
        harness.state().camera.clone().unwrap().to_scene(),
        written_camera
    );
    harness.get_by_label("Show").click();
    harness.run();
    let state = harness.state();
    assert_same_view(&state.camera.clone().unwrap().to_scene(), &written_camera);
    assert_eq!(state.visible, [true, false]);
    assert_eq!(state.selected, Some(0));
    assert_eq!(state.style, DisplayStyle::ShadedEdges);
    assert_eq!(state.section, None);
    let model = state.model.as_ref().expect("model");
    assert_eq!(
        dimensions::world_point(model, &arrow.tip, &state.visible),
        Some(tip_world),
        "orbiting and switching views never move the arrow off its part"
    );

    // Edit keeps the comment's identity; the package itself is never edited.
    harness.get_by_label("Edit").click();
    harness.run();
    harness.get_by_label("Comment").focus();
    harness.run();
    harness.get_by_label("Comment").type_text(" please");
    harness.run();
    harness.get_by_label("Save comment").click();
    harness.run();
    let comments = harness.state().review.as_ref().unwrap().comments.clone();
    assert_eq!(comments.comments.len(), 1);
    assert_eq!(comments.comments[0].id, id);
    assert!(comments.comments[0].text.ends_with("please"));
    assert_eq!(comments.comments[0].arrows, comment.arrows);

    // Closing with unsaved comments warns once.
    assert!(!harness.state_mut().may_discard_comments());
    assert!(harness.state_mut().may_discard_comments());

    let mut returned = Vec::new();
    let written = harness
        .state()
        .save_comments(&mut returned)
        .expect("comments file");
    let mut copy = Vec::new();
    let copied = harness
        .state()
        .save_commented_copy(&mut copy)
        .expect("copy");
    assert!(
        harness.state().has_unsaved_comments(),
        "saved only once the host keeps the file"
    );
    harness.state_mut().comments_written(written);
    harness.state_mut().comments_written(copied);
    assert!(!harness.state().has_unsaved_comments());
    assert_eq!(
        harness.state().model.as_ref().unwrap().package(),
        &original,
        "comments never change the received package"
    );
    assert_eq!(
        &copy[..bytes.len()],
        bytes.as_slice(),
        "the copy carries the same export"
    );

    // Offline reopen: the commented copy, or the package and its return file.
    let mut reopened = ViewerApp::new(None).with_language(Language::ENGLISH);
    reopened.open(copy.as_slice()).expect("commented copy");
    assert_eq!(reopened.review.as_ref().unwrap().comments, comments);
    let mut author = ViewerApp::new(None).with_language(Language::ENGLISH);
    author.open(bytes.as_slice()).expect("package");
    author
        .open_comments(returned.as_slice())
        .expect("return file");
    assert_eq!(author.review.as_ref().unwrap().comments, comments);
    author.show_comment(id).expect("show");
    assert_same_view(&author.camera.clone().unwrap().to_scene(), &written_camera);

    // Comments for another export are refused; the current ones stay.
    let mut other = scene_package();
    other.manifest.definitions[0].note = Some("re-exported".into());
    let mut other_bytes = Vec::new();
    other.write(&mut other_bytes).unwrap();
    let mut stranger = ViewerApp::new(None).with_language(Language::ENGLISH);
    stranger.open(other_bytes.as_slice()).unwrap();
    let refused = stranger.open_comments(returned.as_slice()).unwrap_err();
    assert!(
        stranger
            .review
            .as_ref()
            .unwrap()
            .comments
            .comments
            .is_empty()
    );
    // The model stays on screen, so the headline blames the file, not the model.
    stranger.report(refused);
    let mut shown = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), stranger);
    shown.run();
    shown.get_by_label("The file could not be opened or saved.");
    assert!(shown.query_by_label("The model cannot be shown.").is_none());

    // Delete removes only that comment.
    harness.run();
    harness.get_by_label("Delete").click();
    harness.run();
    assert!(
        harness
            .state()
            .review
            .as_ref()
            .unwrap()
            .comments
            .comments
            .is_empty()
    );
    assert!(harness.state().has_unsaved_comments());
}

#[test]
fn system_opened_comments_file_finds_its_own_package_in_the_folder() {
    let directory =
        std::env::temp_dir().join(format!("ketchup-viewer-open-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let original = scene_package();
    let mut other = scene_package();
    other.manifest.definitions[0].note = Some("re-exported".into());
    let mut comments = ketchup_view_format::Comments::new(&original).unwrap();
    comments.comments.push(ketchup_view_format::Comment {
        id: 1,
        author: Some("Jana".into()),
        text: "Raise the shelf".into(),
        view: CommentView {
            camera: original.manifest.scenes[0].camera.clone(),
            hidden: vec![],
            style: DisplayStyle::Shaded,
            section: None,
            scene: None,
            shown: None,
        },
        selected: Some(original.manifest.occurrences[0].path.clone()),
        arrows: vec![],
    });
    let returned = directory.join("returned.ketchup-comments");
    comments
        .write(std::fs::File::create(&returned).unwrap(), &original)
        .unwrap();
    // A different export of the same project sits next to it; never matched.
    other
        .write(std::fs::File::create(directory.join("a-other.ketchup-view")).unwrap())
        .unwrap();

    let mut app = ViewerApp::new(None).with_language(Language::ENGLISH);
    let error = app.open_path(&returned).unwrap_err();
    assert_eq!(error.code(), "viewer.comments.package");
    assert!(app.model.is_none(), "nothing is opened without its package");

    original
        .write(std::fs::File::create(directory.join("b-shelf.ketchup-view")).unwrap())
        .unwrap();
    app.open_path(&returned)
        .expect("comments with their package");
    assert_eq!(app.model.as_ref().unwrap().package(), &original);
    assert_eq!(app.review.as_ref().unwrap().comments, comments);
    assert_eq!(
        app.opened_counts(),
        Some([
            original.manifest.occurrences.len(),
            original.manifest.scenes.len(),
            1
        ])
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn only_a_sharing_host_offers_sending_comments_to_the_author() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    for (app, offered) in [
        (ViewerApp::new(None).with_host_files(), false),
        (ViewerApp::new(None).with_host_sharing(), true),
    ] {
        let mut app = app.with_language(Language::ENGLISH);
        app.open(bytes.as_slice()).expect("open");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
        harness.run();
        let button = harness.query_by_label("Send comments to the author");
        assert_eq!(button.is_some(), offered);
        if let Some(button) = button {
            button.click();
            harness.run();
            assert_eq!(
                harness.state_mut().take_file_requests(),
                [FileRequest::ShareComments]
            );
        }
    }
}

#[test]
fn web_host_downloads_comments_that_a_fresh_viewer_imports() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = ViewerApp::new(None)
        .with_language(Language::ENGLISH)
        .with_host_files();
    app.open(bytes.as_slice()).expect("open");
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();
    harness.get_by_label("Add comment").click();
    harness.run();
    harness.get_by_label("Comment").focus();
    harness.run();
    harness
        .get_by_label("Comment")
        .type_text("Darker oak, please");
    harness.run();
    harness.get_by_label("Save comment").click();
    harness.run();

    // Unsaved comments: the first Open only warns, the second asks the host.
    harness.get_by_label("Open model").click();
    harness.run();
    assert!(harness.state_mut().take_file_requests().is_empty());
    harness.get_by_label("Open model").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::OpenPackage]
    );
    harness.get_by_label("Save comments for the author").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::SaveComments]
    );

    // VW-1: a save dialog the customer cancels writes nothing, so the
    // comments stay unsaved and the next Open still warns first.
    let mut cancelled = Vec::new();
    let _dropped = harness.state().save_comments(&mut cancelled).unwrap();
    assert!(harness.state().has_unsaved_comments());

    // The host downloads what the Viewer writes; another browser imports it.
    let mut downloaded = Vec::new();
    let written = harness
        .state()
        .save_comments(&mut downloaded)
        .expect("download");
    harness.state_mut().comments_written(written);
    assert!(!harness.state().has_unsaved_comments());
    let mut imported = ViewerApp::new(None).with_host_files();
    imported.open(bytes.as_slice()).expect("open");
    imported
        .open_comments(downloaded.as_slice())
        .expect("import");
    let comments = &imported.review.as_ref().unwrap().comments.comments;
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, "Darker oak, please");
    assert_eq!(imported.opened_counts().map(|[_, _, c]| c), Some(1));
}

/// VW-4 and VW-5: opening a comments file over unsaved comments, and leaving
/// a comment half written, warn first like opening another model.
#[test]
fn opening_comments_or_a_model_over_unsaved_work_warns_first() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let app = customer_with_comment(&bytes, "Darker oak").with_language(Language::ENGLISH);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();

    // VW-4: unsaved comments; the first Open comments only warns.
    harness.get_by_label("Open comments").click();
    harness.run();
    assert!(harness.state_mut().take_file_requests().is_empty());
    assert_eq!(
        harness.state().error.as_ref().map(Rejection::code),
        Some("viewer.comments.unsaved")
    );
    harness.get_by_label("Open comments").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::OpenComments]
    );
    let written = harness.state().save_comments(&mut Vec::new()).unwrap();
    harness.state_mut().comments_written(written);
    harness.state_mut().error = None;
    harness.run();

    // Saved comments open another file at once.
    harness.get_by_label("Open comments").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::OpenComments]
    );

    // VW-5: a comment being written counts as unsaved work.
    harness.get_by_label("Add comment").click();
    harness.run();
    assert!(
        !harness.state().has_unsaved_comments(),
        "an empty new comment loses nothing"
    );
    harness.get_by_label("Comment").focus();
    harness.run();
    harness.get_by_label("Comment").type_text("Brass handles");
    harness.run();
    assert!(harness.state().has_unsaved_comments());
    harness.get_by_label("Open model").click();
    harness.run();
    assert!(harness.state_mut().take_file_requests().is_empty());
    // Writing on after the warning warns again before it is lost.
    harness.get_by_label("Comment").focus();
    harness.run();
    harness.get_by_label("Comment").type_text(" too");
    harness.run();
    let draft = harness.state().review.as_ref().unwrap().draft.as_ref();
    assert_eq!(draft.unwrap().text, "Brass handles too");
    harness.get_by_label("Open model").click();
    harness.run();
    assert!(harness.state_mut().take_file_requests().is_empty());
    harness.get_by_label("Open model").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::OpenPackage]
    );

    // Editing a comment without changing it loses nothing.
    harness.get_by_label("Cancel").click();
    harness.run();
    harness.get_by_label("Edit").click();
    harness.run();
    assert!(!harness.state().has_unsaved_comments());
    harness.get_by_label("Open model").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::OpenPackage]
    );
}

/// A customer's Viewer with one committed comment `text`.
fn customer_with_comment(bytes: &[u8], text: &str) -> ViewerApp {
    let mut app = ViewerApp::new(None).with_host_files();
    app.open(bytes).expect("open");
    let model = app.model.as_ref().unwrap();
    let view = CommentView {
        camera: model.manifest().scenes[0].camera.clone(),
        hidden: vec![],
        style: DisplayStyle::Shaded,
        section: None,
        scene: None,
        shown: None,
    };
    let review = app.review.as_mut().unwrap();
    review.start(view, None);
    review.draft.as_mut().unwrap().text = text.to_owned();
    assert!(review.commit());
    app
}

#[test]
fn comments_count_as_saved_only_for_the_file_the_host_kept() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = customer_with_comment(&bytes, "Darker oak");
    let mut first = Vec::new();
    let written = app.save_comments(&mut first).unwrap();
    // The customer adds a comment while the save dialog is open: keeping
    // the earlier file does not save the newer comment.
    let review = app.review.as_mut().unwrap();
    review.start(review.comments.comments[0].view.clone(), None);
    review.draft.as_mut().unwrap().text = "And brass handles".into();
    assert!(review.commit());
    app.comments_written(written);
    assert!(app.has_unsaved_comments());
    // A file written from another opened package saves nothing here.
    let other = customer_with_comment(&bytes, "Other");
    let foreign = other.save_comments(&mut Vec::new()).unwrap();
    app.comments_written(foreign);
    assert!(app.has_unsaved_comments());
    let current = app.save_comments(&mut Vec::new()).unwrap();
    app.comments_written(current);
    assert!(!app.has_unsaved_comments());
}

/// VW-16: the shared link's download finishing after the customer opened a
/// local package and commented on it leaves both, and so does its failure;
/// a download nothing superseded opens.
#[test]
fn a_late_shared_download_never_replaces_the_package_opened_since() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = ViewerApp::new(None).with_host_files();
    let late = app.begin_open();
    let failed_late = app.begin_open();
    app.open(bytes.as_slice()).expect("open");
    let view = CommentView {
        camera: app.model.as_ref().unwrap().manifest().scenes[0]
            .camera
            .clone(),
        hidden: vec![],
        style: DisplayStyle::Shaded,
        section: None,
        scene: None,
        shown: None,
    };
    let review = app.review.as_mut().unwrap();
    review.start(view, None);
    review.draft.as_mut().unwrap().text = "Darker oak".to_owned();
    assert!(review.commit());

    assert_eq!(app.finish_open(late, bytes.as_slice()), Ok(false));
    assert!(app.has_unsaved_comments());
    assert_eq!(app.opened_counts().map(|[.., comments]| comments), Some(1));
    app.fail_open(
        failed_late,
        Rejection::new("viewer.web.fetch", RejectionPhase::Io),
    );
    assert!(app.error().is_none());
    assert!(!app.loading);

    let mut fresh = ViewerApp::new(None).with_host_files();
    let pending = fresh.begin_open();
    assert!(fresh.loading);
    assert_eq!(fresh.finish_open(pending, bytes.as_slice()), Ok(true));
    assert!(!fresh.loading && fresh.opened_counts().is_some());
}

/// Keeps `app` in `keeper` as the phone does frame after frame, until the
/// package written on its own thread is on disk and its comments follow it.
fn keep_all(keeper: &mut SessionKeeper, app: &mut ViewerApp) -> Result<(), Rejection> {
    keeper.keep(app)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while keeper.writing() {
        assert!(
            std::time::Instant::now() < deadline,
            "keeper.writing() still true after 30 s: the package write never finished"
        );
        std::thread::yield_now();
        keeper.keep(app)?;
    }
    Ok(())
}

/// Adds the comment `text` beside the first one.
fn add_comment(app: &mut ViewerApp, text: &str) {
    let review = app.review.as_mut().unwrap();
    review.start(review.comments.comments[0].view.clone(), None);
    review.draft.as_mut().unwrap().text = text.into();
    assert!(review.commit());
}

/// VW-2: Android ends a Viewer in the background (and used to on split
/// screen, a fold, a keyboard or a language change); the next start opens the
/// model and comments it kept, unsaved ones still unsaved.
#[test]
fn a_viewer_ended_in_the_background_reopens_its_model_and_unsaved_comments() {
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-session-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = customer_with_comment(&bytes, "Darker oak");
    let mut keeper = SessionKeeper::new(dir.clone(), None);
    keep_all(&mut keeper, &mut ViewerApp::new(None)).unwrap();
    assert!(!dir.exists(), "nothing open, nothing kept");
    keep_all(&mut keeper, &mut app).unwrap();
    // Unchanged, nothing is written again.
    std::fs::remove_file(dir.join("session.ketchup-view")).unwrap();
    keep_all(&mut keeper, &mut app).unwrap();
    assert!(!dir.join("session.ketchup-view").exists());

    let mut keeper = SessionKeeper::new(dir.clone(), None);
    keep_all(&mut keeper, &mut app).unwrap();
    let mut restarted = ViewerApp::new(None).with_host_files();
    assert_eq!(
        restarted.restore_session(&dir).unwrap(),
        restarted.session_mark()
    );
    assert_eq!(restarted.opened_counts(), app.opened_counts());
    let text = |app: &ViewerApp| {
        app.review.as_ref().unwrap().comments.comments[0]
            .text
            .clone()
    };
    assert_eq!(text(&restarted), "Darker oak");
    assert!(restarted.has_unsaved_comments());
    // A second comment, then a save the host kept: the session follows both.
    add_comment(&mut app, "And brass handles");
    keep_all(&mut keeper, &mut app).unwrap();
    let written = app.save_comments(&mut Vec::new()).unwrap();
    app.comments_written(written);
    keep_all(&mut keeper, &mut app).unwrap();
    let mut restarted = ViewerApp::new(None);
    restarted.restore_session(&dir).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(2));
    assert!(!restarted.has_unsaved_comments());
    // Nothing kept: nothing opens.
    assert_eq!(
        ViewerApp::new(None)
            .restore_session(&dir.join("none"))
            .unwrap(),
        None
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// VW-10: the session keeps the package once, when it is opened, on a thread
/// of its own, and its comments after it; a comment edit writes only the
/// comments, so a large package is not written again at every edit. Another
/// package drops the kept comments.
#[test]
fn a_comment_edit_keeps_only_the_comments_not_the_whole_package() {
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = customer_with_comment(&bytes, "Darker oak");
    let mut keeper = SessionKeeper::new(dir.clone(), None);
    keeper.keep(&mut app).unwrap();
    assert!(
        keeper.writing(),
        "the package is written apart from the frame"
    );
    assert!(dir.join("session.ketchup-comments").exists());
    keep_all(&mut keeper, &mut app).unwrap();
    let package = dir.join("session.ketchup-view");
    assert_eq!(std::fs::read(&package).unwrap(), bytes);
    std::fs::rename(&package, dir.join("aside")).unwrap();

    add_comment(&mut app, "And brass handles");
    keeper.keep(&mut app).unwrap();
    assert!(!keeper.writing(), "the package was written again");
    assert!(!package.exists(), "the package was written again");
    assert!(dir.join("session.ketchup-comments").exists());
    std::fs::rename(dir.join("aside"), &package).unwrap();
    let mut restarted = ViewerApp::new(None);
    assert!(restarted.restore_session(&dir).unwrap().is_some());
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(2));
    assert!(restarted.has_unsaved_comments());

    // Another package: written, and the first one's comments are gone.
    let mut other = scene_package();
    other.manifest.source.revision += 1;
    let mut other_bytes = Vec::new();
    other.write(&mut other_bytes).expect("other");
    let mut app = customer_with_comment(&other_bytes, "Other");
    keep_all(&mut keeper, &mut app).unwrap();
    assert_eq!(std::fs::read(&package).unwrap(), other_bytes);
    let mut restarted = ViewerApp::new(None);
    restarted.restore_session(&dir).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(1));

    // A session kept before comments had their own file: a commented copy.
    let mut copy = Vec::new();
    app.save_commented_copy(&mut copy).unwrap();
    std::fs::remove_file(dir.join("session.ketchup-comments")).unwrap();
    std::fs::write(&package, &copy).unwrap();
    let mut restarted = ViewerApp::new(None);
    restarted.restore_session(&dir).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(1));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Review 2026-10-17 D-7: Android may end the process while a large package
/// is written on its own thread. The comments made meanwhile are on disk
/// beside the mark of that package at once, not waiting for it, so they open
/// with it at the next start.
#[test]
fn comments_made_while_the_package_is_written_survive_the_end_of_the_process() {
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-ended-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = customer_with_comment(&bytes, "Darker oak");
    let mut keeper = SessionKeeper::new(dir.clone(), None);
    keeper.keep(&mut app).unwrap();
    let comments = dir.join("session.ketchup-comments");
    assert!(comments.exists(), "the comments wait for the package");
    add_comment(&mut app, "And brass handles");
    keeper.keep(&mut app).unwrap();
    // The process ends: no further keep, only the package writer finishes.
    drop(keeper);
    let package = dir.join("session.ketchup-view");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !package.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the package was never written"
        );
        std::thread::yield_now();
    }
    let mut restarted = ViewerApp::new(None);
    assert!(restarted.restore_session(&dir).unwrap().is_some());
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(2));
    assert!(restarted.has_unsaved_comments());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Held by [`a_phone_reads_its_start_model_apart_from_the_frame`] to keep the
/// startup reader from reading.
static STARTUP_READER: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn hold_startup_reader() {
    drop(
        STARTUP_READER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
}

/// Review 2026-10-17 §3 P3: a phone's start reads the kept session, else the
/// installed package, on a thread of its own, not in the first frame on the
/// UI thread; meanwhile the Viewer is empty and says the model is loading.
/// Then it shows what restoring the session in the frame showed.
#[test]
fn a_phone_reads_its_start_model_apart_from_the_frame() {
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-startup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = customer_with_comment(&bytes, "Darker oak");
    keep_all(&mut SessionKeeper::new(dir.clone(), None), &mut app).unwrap();

    let held = STARTUP_READER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut started = ViewerApp::new(None).with_host_files();
    let opening = StartupOpen::start(&mut started, Some(dir.clone()), None);
    assert!(!opening.is_finished(), "the start waits for no reading");
    assert!(started.loading && started.model.is_none());
    drop(held);
    let (mark, failures) = opening.finish(&mut started);
    assert!(failures.is_empty(), "{failures:?}");
    assert!(!started.loading);
    assert!(mark.is_some());
    assert_eq!(mark, started.session_mark());
    assert_eq!(started.opened_counts(), app.opened_counts());
    assert!(started.has_unsaved_comments());

    // A kept session that does not open is reported; the installed package opens.
    std::fs::write(dir.join("session.ketchup-view"), b"not a package").unwrap();
    let installed = dir.join("model.ketchup-view");
    std::fs::write(&installed, &bytes).unwrap();
    let mut fresh = ViewerApp::new(None);
    let opening = StartupOpen::start(&mut fresh, Some(dir.clone()), Some(installed));
    let (mark, failures) = opening.finish(&mut fresh);
    assert_eq!((mark, failures.len()), (None, 1));
    let shown = |app: &ViewerApp| {
        app.opened_counts()
            .map(|[components, scenes, _]| [components, scenes])
    };
    assert!(shown(&fresh).is_some());
    assert_eq!(shown(&fresh), shown(&app));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Holds the package writer for good in the child process of
/// [`comments_beside_a_package_whose_writer_was_killed_open_with_it_again`],
/// so it is killed with its `.part` not renamed.
static HOLD_PACKAGE_WRITER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn hold_package_writer() {
    if HOLD_PACKAGE_WRITER.load(std::sync::atomic::Ordering::SeqCst) {
        loop {
            std::thread::park();
        }
    }
}

/// The session folder the killed child keeps its session in.
const KILLED_SESSION: &str = "KETCHUP_VIEWER_KILLED_SESSION";
/// What the child prints once it may be killed.
const KILL_NOW: &str = "kill-the-viewer-now";

/// Package A and another export B of it.
fn two_packages() -> (Vec<u8>, Vec<u8>) {
    let mut first = Vec::new();
    scene_package().write(&mut first).expect("fixture");
    let mut other = scene_package();
    other.manifest.source.revision += 1;
    let mut second = Vec::new();
    other.write(&mut second).expect("other");
    (first, second)
}

/// The child of the next test: keeps A with its comments saved, opens B,
/// comments on it and waits to be killed while B's writer holds its `.part`.
#[test]
#[ignore = "run in a child process by comments_beside_a_package_whose_writer_was_killed_open_with_it_again"]
fn session_with_a_package_writer_to_kill() {
    let Some(dir) = std::env::var_os(KILLED_SESSION) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let (first, second) = two_packages();
    let mut app = customer_with_comment(&first, "Darker oak");
    let mut keeper = SessionKeeper::new(dir, None);
    keep_all(&mut keeper, &mut app).unwrap();
    let written = app.save_comments(&mut Vec::new()).unwrap();
    app.comments_written(written);
    keep_all(&mut keeper, &mut app).unwrap();

    HOLD_PACKAGE_WRITER.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut app = customer_with_comment(&second, "Other");
    keeper.keep(&mut app).unwrap();
    add_comment(&mut app, "And brass handles");
    keeper.keep(&mut app).unwrap();
    assert!(keeper.writing());
    println!("\n{KILL_NOW}");
    loop {
        std::thread::park();
    }
}

/// Review 2026-10-17 VW-12: the process is killed while package B is
/// written, so B's `.part` is never renamed and the next start opens A. B's
/// unsaved marker does not mark A's comments, and opening B again brings
/// back the comments kept beside it instead of dropping them.
#[test]
fn comments_beside_a_package_whose_writer_was_killed_open_with_it_again() {
    use std::io::BufRead as _;
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-killed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app::tests::session_with_a_package_writer_to_kill",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(KILLED_SESSION, &dir)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let ready = stdout
        .lines()
        .map_while(Result::ok)
        .any(|line| line == KILL_NOW);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(ready, "the child ended before B was being written");
    assert!(dir.join("session.ketchup-view.part").exists());
    let (first, second) = two_packages();
    assert_eq!(
        std::fs::read(dir.join("session.ketchup-view")).unwrap(),
        first
    );

    let mut restarted = ViewerApp::new(None);
    let mark = restarted.restore_session(&dir).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(0));
    assert!(
        !restarted.has_unsaved_comments(),
        "B's unsaved marker marked A's comments"
    );
    let mut keeper = SessionKeeper::new(dir.clone(), mark);
    keep_all(&mut keeper, &mut restarted).unwrap();

    // The customer opens B again: its two unsaved comments are back.
    restarted.open(second.as_slice()).unwrap();
    keep_all(&mut keeper, &mut restarted).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(2));
    assert!(restarted.has_unsaved_comments());
    let mut again = ViewerApp::new(None);
    again.restore_session(&dir).unwrap();
    assert_eq!(again.opened_counts().map(|[_, _, c]| c), Some(2));
    assert!(again.has_unsaved_comments());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Review 2026-10-15 N-11: keeping another package fails (a full device);
/// the phone does not retry every frame. The next comment then writes that
/// package again instead of keeping its comments beside the previous
/// package, and comments that do not open on the kept package count as none
/// rather than losing the package at the next start.
#[test]
fn a_session_that_failed_to_keep_a_package_writes_it_with_the_next_change() {
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-full-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut first = customer_with_comment(&bytes, "Darker oak");
    let mut keeper = SessionKeeper::new(dir.clone(), None);
    keep_all(&mut keeper, &mut first).unwrap();

    let mut other = scene_package();
    other.manifest.source.revision += 1;
    let mut other_bytes = Vec::new();
    other.write(&mut other_bytes).expect("other");
    let mut app = customer_with_comment(&other_bytes, "Other");
    let blocked = dir.join("session.ketchup-view.part");
    std::fs::create_dir(&blocked).unwrap();
    assert!(keep_all(&mut keeper, &mut app).is_err());
    keeper.keep(&mut app).unwrap();
    assert!(!keeper.writing(), "retried");
    std::fs::remove_dir(&blocked).unwrap();

    add_comment(&mut app, "And brass handles");
    keep_all(&mut keeper, &mut app).unwrap();
    let package = dir.join("session.ketchup-view");
    assert_eq!(std::fs::read(&package).unwrap(), other_bytes);
    let mut restarted = ViewerApp::new(None);
    restarted.restore_session(&dir).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(2));

    // The other package's comments beside the first package: it still opens.
    std::fs::write(&package, &bytes).unwrap();
    let mut restarted = ViewerApp::new(None);
    assert!(restarted.restore_session(&dir).unwrap().is_some());
    assert_eq!(
        restarted.opened_counts(),
        first.opened_counts().map(|[a, b, _]| [a, b, 0])
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Review 2026-10-16 status: a comments write that fails keeps the package
/// kept beside them, so the next edit writes only the comments again, not
/// the whole package.
#[test]
fn a_failed_comments_write_does_not_write_the_package_again() {
    let dir = std::env::temp_dir().join(format!("ketchup-viewer-comments-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let mut app = customer_with_comment(&bytes, "Darker oak");
    let mut keeper = SessionKeeper::new(dir.clone(), None);
    keep_all(&mut keeper, &mut app).unwrap();

    let blocked = dir.join("session.ketchup-comments.part");
    std::fs::create_dir(&blocked).unwrap();
    add_comment(&mut app, "And brass handles");
    assert!(keeper.keep(&mut app).is_err());
    keeper.keep(&mut app).unwrap();
    std::fs::remove_dir(&blocked).unwrap();

    add_comment(&mut app, "Soft-close hinges");
    keeper.keep(&mut app).unwrap();
    assert!(!keeper.writing(), "the package was written again");
    let mut restarted = ViewerApp::new(None);
    restarted.restore_session(&dir).unwrap();
    assert_eq!(restarted.opened_counts().map(|[_, _, c]| c), Some(3));
    assert!(restarted.has_unsaved_comments());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Review 2026-10-17 VW-13: a host that keeps the earlier content of a file
/// whose cut-off save it gave up offers saving it where the customer can
/// reach it; the offer goes once the host keeps nothing more.
#[test]
fn earlier_content_the_host_keeps_is_offered_for_saving_elsewhere() {
    let app = ViewerApp::new(None)
        .with_host_files()
        .with_language(Language::ENGLISH);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
    harness.run();
    let label = "Save earlier content as…";
    assert!(harness.query_by_label(label).is_none());
    harness.state_mut().offer_kept(true);
    harness.run();
    harness.get_by_label(label).click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_file_requests(),
        [FileRequest::SaveKept]
    );
    harness.state_mut().offer_kept(false);
    harness.run();
    assert!(harness.query_by_label(label).is_none());
}

/// VW-7: two customers comment on one export; the author keeps both,
/// because the author matches comments by export and ID.
#[test]
fn two_customers_comments_on_one_export_have_different_ids() {
    let mut bytes = Vec::new();
    scene_package().write(&mut bytes).expect("fixture");
    let jana = customer_with_comment(&bytes, "Raise the shelf");
    let peter = customer_with_comment(&bytes, "Lower the shelf");
    let id = |app: &ViewerApp| app.review.as_ref().unwrap().comments.comments[0].id;
    assert_ne!(id(&jana), id(&peter));
    assert!(id(&jana) < 1 << 53 && id(&peter) < 1 << 53);
}

#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
mod atomic_save {
    use super::comments_section::write_atomically;
    use ketchup_rejection::{Rejection, RejectionPhase};
    use std::io::Write as _;

    #[test]
    fn interrupted_save_keeps_the_earlier_file_and_leaves_no_partial_one() {
        let directory =
            std::env::temp_dir().join(format!("ketchup-viewer-save-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("model.ketchup-comments");
        std::fs::write(&path, b"earlier").unwrap();
        let failed = write_atomically(&path, |file| {
            file.write_all(b"half").unwrap();
            Err::<(), _>(Rejection::new("test.failure", RejectionPhase::Io))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"earlier");
        assert!(!directory.join("model.ketchup-comments.part").exists());
        write_atomically(&path, |file| {
            file.write_all(b"new")
                .map_err(|error| Rejection::new("test.write", RejectionPhase::Io).caused_by(error))
        })
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
