use super::*;
use egui_kittest::Harness;

fn key(app: &mut KetchupApp, key: egui::Key) {
    let context = egui::Context::default();
    let _ = context.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |context| app.handle_shortcuts(context),
    );
}

fn planes() -> [(egui::Key, PrincipalPlane); 3] {
    [
        (egui::Key::ArrowRight, PrincipalPlane::Yz),
        (egui::Key::ArrowLeft, PrincipalPlane::Xz),
        (egui::Key::ArrowUp, PrincipalPlane::Xy),
    ]
}

fn shell(app: KetchupApp) -> Harness<'static, KetchupApp> {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    harness
}

fn pointer(harness: &mut Harness<'_, KetchupApp>, world: Vec3) -> Pos2 {
    let screen = harness.state().viewport_position(world).unwrap();
    assert!(
        harness.state().viewport_rect.unwrap().contains(screen),
        "{world:?} projects outside viewport at {screen:?} ({:?})",
        harness.state().projection_mode
    );
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(screen));
    harness.step();
    screen
}

fn click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    let screen = pointer(harness, world);
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: screen,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
}

fn press(harness: &mut Harness<'_, KetchupApp>, key: egui::Key) {
    harness.key_press(key);
    harness.step();
}

fn source() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.new_document();
    assert!(app.create_profile_at(
        Vec3::new(20.0, 10.0, 40.0),
        vec![[0.0, 0.0], [60.0, 0.0], [60.0, 50.0], [0.0, 50.0]]
    ));
    app
}

fn near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < 1e-4,
        "expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn drawing_plane_first_point_accepts_z_axis() {
    for command in [AppCommand::Rectangle, AppCommand::Circle, AppCommand::Arc] {
        for projection in [ProjectionMode::Parallel, ProjectionMode::Perspective] {
            let mut app = KetchupApp::new();
            app.new_document();
            app.projection_mode = projection;
            app.dispatch_command(command);
            let mut harness = shell(app);
            let expected = Vec3::new(0.0, 0.0, 25.0);
            click(&mut harness, expected);
            near(harness.state().sketch_start.unwrap(), expected);
        }
    }
}

#[test]
fn drawing_plane_arrows_anchor_every_plane_at_object_corner() {
    for (key_code, plane) in planes() {
        let mut app = source();
        let start = Vec3::new(80.0, 60.0, 40.0);
        app.dispatch_command(AppCommand::Rectangle);
        key(&mut app, key_code);
        assert_eq!(app.face_workflow_datum(), plane);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0));
        let pointer = app.project(start, rect);
        app.update_viewport_inference(Some(pointer), rect);
        assert_eq!(app.hovered_snap_position(), Some(start));
        assert_eq!(app.drawing_input_point(pointer, rect), Some(start));
        app.sketch_start = Some(start);
        let expected = app.drawing_world_delta(start, Vec3::new(30.0, 20.0, 0.0));
        let actual = app
            .drawing_input_point(app.project(expected, rect), rect)
            .unwrap();
        near(actual, expected);
        assert!(app.complete_rectangle_sketch(start, actual));
    }
}

#[test]
fn drawing_plane_rectangle_tool_remains_ready_after_completion() {
    let mut app = KetchupApp::new();
    app.new_document();
    app.dispatch_command(AppCommand::Rectangle);
    assert!(app.complete_rectangle_sketch(Vec3::ZERO, Vec3::new(60.0, 40.0, 0.0)));
    assert_eq!(app.active_tool, ActiveTool::Rectangle);
    assert!(
        app.sketch_mode,
        "visible Rectangle tool must accept the next first corner"
    );
    assert!(app.sketch_start.is_none());
}

#[test]
fn drawing_plane_rectangle_headless_preview_commit_repeat_and_history() {
    for (key_code, plane) in planes() {
        let mut app = source();
        app.dispatch_command(AppCommand::Rectangle);
        let mut harness = shell(app);
        let start = Vec3::new(80.0, 60.0, 40.0);
        click(&mut harness, start);
        near(harness.state().sketch_start.unwrap(), start);
        press(&mut harness, key_code);
        assert_eq!(harness.state().face_workflow_datum(), plane);
        near(harness.state().sketch_start.unwrap(), start);
        let end = harness
            .state()
            .drawing_world_delta(start, Vec3::new(30.0, 25.0, 0.0));
        let before = harness.state().canonical_digest();
        let undo = harness.state().undo_step_count();
        pointer(&mut harness, end);
        near(harness.state().sketch_cursor.unwrap(), end);
        let corners = harness.state().drawing_rectangle_corners(start, end);
        let pixels = corners.map(|p| harness.state().viewport_position(p).unwrap());
        for i in 0..4 {
            assert!(harness.output().shapes.iter().any(|s| matches!(&s.shape,
                egui::Shape::LineSegment { points, stroke } if points[0].distance(pixels[i]) < 0.01 && points[1].distance(pixels[(i+1)%4]) < 0.01 && stroke.color == Color32::from_rgb(255,199,68)
            )), "missing preview edge {i} in {plane:?}");
        }
        assert_eq!(harness.state().canonical_digest(), before);
        click(&mut harness, end);
        assert_eq!(harness.state().undo_step_count(), undo + 1);
        let after = harness.state().canonical_digest();
        let rect = harness.state().viewport_rect.unwrap();
        for corner in corners {
            near(
                harness
                    .state()
                    .scene_snap_at_screen(harness.state().project(corner, rect), rect, 8.0, None)
                    .unwrap()
                    .position_mm,
                corner,
            );
        }
        click(&mut harness, end);
        near(harness.state().sketch_start.unwrap(), end);
        press(&mut harness, egui::Key::Escape);
        assert_eq!(harness.state().canonical_digest(), after);
        assert!(harness.state_mut().undo());
        assert_eq!(harness.state().canonical_digest(), before);
        assert!(harness.state_mut().redo());
        assert_eq!(harness.state().canonical_digest(), after);
    }
}

#[test]
fn drawing_plane_circle_and_arc_preview_equal_commit_on_all_planes() {
    for (key_code, plane) in planes() {
        for command in [AppCommand::Circle, AppCommand::Arc] {
            let mut app = source();
            app.dispatch_command(command);
            let mut harness = shell(app);
            let start = Vec3::new(80.0, 60.0, 40.0);
            click(&mut harness, start);
            press(&mut harness, key_code);
            assert_eq!(harness.state().face_workflow_datum(), plane);
            let end = harness
                .state()
                .drawing_world_delta(start, Vec3::new(30.0, 0.0, 0.0));
            let before = harness.state().canonical_digest();
            let undo = harness.state().undo_step_count();
            pointer(&mut harness, end);
            let expected_arc = if command == AppCommand::Arc {
                click(&mut harness, end);
                let bulge = harness
                    .state()
                    .drawing_world_delta(start, Vec3::new(15.0, 10.0, 0.0));
                pointer(&mut harness, bulge);
                let arc = harness.state().arc_preview_geometry().unwrap();
                assert_eq!(harness.state().canonical_digest(), before);
                click(&mut harness, bulge);
                Some(arc)
            } else {
                assert_eq!(harness.state().value_input(), "30");
                click(&mut harness, end);
                None
            };
            assert_eq!(harness.state().undo_step_count(), undo + 1);
            let snapshot = harness.state().document.current();
            let occurrence = snapshot.occurrences().last().unwrap();
            let transform = occurrence.transform();
            let m = transform.matrix();
            let f = WorkplaneFrame::principal(plane);
            near(
                Vec3::new(m[2], m[6], m[10]),
                Vec3::new(f.normal[0], f.normal[1], f.normal[2]),
            );
            if let Some((a, b, c, cw)) = expected_arc {
                let (aa, bb, cc, ccw) = harness.state().latest_arc_geometry().unwrap();
                near(aa, a);
                near(bb, b);
                near(cc, c);
                assert_eq!(ccw, cw);
            } else {
                let (center, radius) = harness.state().latest_circle_geometry().unwrap();
                near(center, start);
                assert!((radius - 30.0).abs() < 1e-4);
            }
            let after = harness.state().canonical_digest();
            assert!(harness.state_mut().undo());
            assert_eq!(harness.state().canonical_digest(), before);
            assert!(harness.state_mut().redo());
            assert_eq!(harness.state().canonical_digest(), after);
        }
    }
}

#[test]
fn drawing_plane_rotation_axis_keys_preserve_typed_angle() {
    let mut app = KetchupApp::new();
    app.headless_select_occurrence(OccurrenceId(1));
    app.dispatch_command(AppCommand::Rotate);
    let mut harness = shell(app);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("90".into()));
    harness.step();
    harness.step();
    assert_eq!(harness.state().value_input(), "90");
    press(&mut harness, egui::Key::ArrowRight);
    assert_eq!(harness.state().rotate_axis_lock, Some(Axis::X));
    assert_eq!(harness.state().value_input(), "90");
    press(&mut harness, egui::Key::Enter);
    let size = harness.state().occurrence_box_geometry(1).unwrap().1;
    near(size, Vec3::new(100.0, 20.0, 60.0));
}
#[test]
fn drawing_plane_axis_keys_do_not_steal_foreign_text_focus() {
    let mut app = KetchupApp::new();
    app.dispatch_command(AppCommand::Rotate);
    let context = egui::Context::default();
    let mut text = String::from("Other editor");
    let _ = context.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new("foreign-editor")))
                .request_focus();
        });
    });
    let _ = context.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ctx| {
            assert!(ctx.wants_keyboard_input());
            app.handle_shortcuts(ctx);
        },
    );
    assert_eq!(app.rotate_axis_lock, None);
}
#[test]
fn drawing_plane_axis_keys_are_shared_and_toggle_for_all_free_tools() {
    for command in [
        AppCommand::Rectangle,
        AppCommand::Circle,
        AppCommand::Arc,
        AppCommand::Line,
        AppCommand::Move,
        AppCommand::Rotate,
    ] {
        let mut app = KetchupApp::new();
        app.dispatch_command(command);
        for (key_code, axis, plane) in [
            (egui::Key::ArrowRight, Axis::X, PrincipalPlane::Yz),
            (egui::Key::ArrowLeft, Axis::Y, PrincipalPlane::Xz),
            (egui::Key::ArrowUp, Axis::Z, PrincipalPlane::Xy),
        ] {
            key(&mut app, key_code);
            match command {
                AppCommand::Line => assert_eq!(app.line_axis_lock, Some(axis)),
                AppCommand::Move => assert_eq!(app.move_axis_lock, Some(axis)),
                AppCommand::Rotate => assert_eq!(app.rotate_axis_lock, Some(axis)),
                _ => assert_eq!(app.face_workflow_datum(), plane),
            };
            key(&mut app, egui::Key::ArrowDown);
            match command {
                AppCommand::Line => assert_eq!(app.line_axis_lock, None),
                AppCommand::Move => assert_eq!(app.move_axis_lock, None),
                AppCommand::Rotate => assert_eq!(app.rotate_axis_lock, None),
                _ => assert_eq!(app.face_workflow_datum(), PrincipalPlane::Xy),
            };
        }
    }
}
