use super::*;
use egui_kittest::Harness;

fn shell() -> Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new();
    app.selection.select_occurrence(OccurrenceId(1), false);
    app.dispatch_command(AppCommand::Rotate);
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    harness
}

fn point(harness: &mut Harness<'_, KetchupApp>, world: Vec3) -> Pos2 {
    let screen = harness.state().viewport_position(world).unwrap();
    assert!(harness.state().viewport_rect.unwrap().contains(screen));
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(screen));
    harness.step();
    screen
}

fn click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    let screen = point(harness, world);
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

fn near(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < 1e-4,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn rotation_click_snaps_pivot_to_corner_without_starting_angle() {
    let mut harness = shell();
    let pivot = Vec3::new(100.0, 60.0, 20.0);
    let before = harness.state().canonical_digest();
    click(&mut harness, pivot);
    let drag = harness.state().active_rotate_gesture().unwrap();
    near(drag.centre_mm, pivot);
    assert!(drag.reference_mm.is_none());
    point(&mut harness, Vec3::new(150.0, 60.0, 20.0));
    assert!(
        harness
            .state()
            .active_rotate_gesture()
            .unwrap()
            .reference_mm
            .is_none(),
        "hover must not silently choose the reference arm"
    );
    assert_eq!(harness.state().canonical_digest(), before);
}

#[test]
fn rotation_selected_object_accepts_external_datum_pivot() {
    let mut harness = shell();
    let pivot = Vec3::new(0.0, 0.0, 80.0);
    click(&mut harness, pivot);
    let drag = harness
        .state()
        .active_rotate_gesture()
        .expect("selected target allows a pivot outside it");
    near(drag.centre_mm, pivot);
    assert_eq!(
        drag.selection.instance_path,
        InstancePath::root(OccurrenceId(1))
    );
}

#[test]
fn rotation_preview_does_not_enable_an_extra_bounding_box() {
    let mut harness = shell();
    click(&mut harness, Vec3::new(100.0, 60.0, 20.0));
    let app = harness.state_mut();
    app.rotate_anchor.as_mut().unwrap().angle_degrees = 37.0;
    let item = app.active_boxes().into_iter().next().unwrap();
    assert!(
        !app.proxy_preview_is_active(&item),
        "rigid rotation already transforms the actual mesh and must not add a box proxy"
    );
    assert_eq!(app.rotate_preview_transform_overrides().len(), 1);
}

#[test]
fn rotation_three_clicks_preserve_pivot_on_every_axis_and_undo_redo() {
    for (axis, arm, end) in [
        (
            Axis::X,
            Vec3::new(0.0, 40.0, 0.0),
            Vec3::new(0.0, 0.0, 40.0),
        ),
        (
            Axis::Y,
            Vec3::new(0.0, 0.0, 40.0),
            Vec3::new(40.0, 0.0, 0.0),
        ),
        (
            Axis::Z,
            Vec3::new(40.0, 0.0, 0.0),
            Vec3::new(0.0, 40.0, 0.0),
        ),
    ] {
        let mut harness = shell();
        let pivot = Vec3::new(100.0, 60.0, 20.0);
        let before = harness.state().canonical_digest();
        let steps = harness.state().undo_step_count();
        click(&mut harness, pivot);
        harness.state_mut().set_rotate_axis_lock(Some(axis));
        near(
            harness.state().active_rotate_gesture().unwrap().centre_mm,
            pivot,
        );
        click(&mut harness, pivot + arm);
        assert_eq!(harness.state().canonical_digest(), before);
        point(&mut harness, pivot + end);
        let preview = harness.state().rotate_preview_transform_overrides()
            [&InstancePath::root(OccurrenceId(1))];
        near(transform_model_point(preview, pivot), pivot);
        near(transform_model_point(preview, pivot + arm), pivot + end);
        click(&mut harness, pivot + end);
        assert_eq!(harness.state().undo_step_count(), steps + 1);
        let after = harness.state().canonical_digest();
        let transform = harness
            .state()
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform();
        assert_eq!(transform, preview);
        harness.state_mut().dispatch_command(AppCommand::Undo);
        assert_eq!(harness.state().canonical_digest(), before);
        harness.state_mut().dispatch_command(AppCommand::Redo);
        assert_eq!(harness.state().canonical_digest(), after);
    }
}

fn painted_body_vertices(app: &mut KetchupApp, context: &egui::Context) -> Vec<(i32, i32)> {
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            ..Default::default()
        },
        |context| app.ui(context),
    );
    let mut points = Vec::new();
    for clipped in output.shapes {
        if let egui::Shape::Mesh(mesh) = clipped.shape {
            for vertex in &mesh.vertices {
                if vertex.color == Color32::from_rgb(154, 91, 67) {
                    points.push((
                        (vertex.pos.x * 100.0).round() as i32,
                        (vertex.pos.y * 100.0).round() as i32,
                    ));
                }
            }
        }
    }
    points.sort_unstable();
    assert!(
        !points.is_empty(),
        "must observe painted model faces: selection={:?}, rect={:?}",
        app.selection.occurrences,
        app.viewport_rect
    );
    points
}

#[test]
fn rotation_rendered_preview_matches_committed_geometry_and_copy() {
    for polygon in [false, true] {
        for copy in [false, true] {
            let mut app = KetchupApp::new();
            if polygon {
                app.new_document();
                assert!(
                    app.create_profile_at(Vec3::ZERO, vec![[0.0, 0.0], [100.0, 0.0], [30.0, 60.0]])
                );
            }
            let id = app.document.current().occurrences().next().unwrap().id();
            app.selection.select_occurrence(id, false);
            app.dispatch_command(AppCommand::Rotate);
            let context = egui::Context::default();
            let _ = context.run(egui::RawInput::default(), |ctx| app.ui(ctx));
            let pivot = Vec3::new(100.0, 60.0, 20.0);
            let rect = app.viewport_rect.unwrap();
            assert!(app.begin_rotate_drag_at(app.project(pivot, rect), rect, copy));
            app.rotate_drag.as_mut().unwrap().angle_degrees = 37.0;
            let digest = app.canonical_digest();
            let preview = painted_body_vertices(&mut app, &context);
            assert_eq!(app.canonical_digest(), digest);
            let drag = app.rotate_drag.take().unwrap();
            assert!(app.commit_rotate_drag(&drag));
            app.selection.clear();
            for occurrence in app.document.current().occurrences() {
                app.selection.select_occurrence(occurrence.id(), true);
            }
            let committed = painted_body_vertices(&mut app, &context);
            assert_eq!(
                preview, committed,
                "polygon={polygon}, copy={copy}: preview must draw actual geometry, not an enclosing box"
            );
        }
    }
}

#[test]
fn rotation_pivot_on_other_object_preserves_selected_target() {
    let mut harness = shell();
    let other = Vec3::new(110.0, 10.0, 40.0);
    assert!(
        harness
            .state_mut()
            .create_box_at(other, Vec3::new(30.0, 20.0, 10.0))
    );
    harness
        .state_mut()
        .selection
        .select_occurrence(OccurrenceId(1), false);
    harness.state_mut().dispatch_command(AppCommand::Rotate);
    click(&mut harness, other + Vec3::new(30.0, 20.0, 10.0));
    let drag = harness.state().active_rotate_gesture().unwrap();
    near(drag.centre_mm, other + Vec3::new(30.0, 20.0, 10.0));
    assert_eq!(
        drag.selection.instance_path,
        InstancePath::root(OccurrenceId(1))
    );
}

#[test]
fn rotation_typed_angle_uses_pivot_and_escape_discards_it() {
    let mut harness = shell();
    let pivot = Vec3::new(100.0, 60.0, 20.0);
    let before = harness.state().canonical_digest();
    click(&mut harness, pivot);
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().active_rotate_gesture().is_none());
    assert_eq!(harness.state().canonical_digest(), before);
    harness.state_mut().dispatch_command(AppCommand::Rotate);
    click(&mut harness, pivot);
    harness.state_mut().value_input = "90".to_owned();
    assert!(harness.state_mut().apply_value_input());
    let transform = harness
        .state()
        .document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform();
    near(transform_model_point(transform, pivot), pivot);
    near(
        transform_model_point(transform, pivot + Vec3::new(40.0, 0.0, 0.0)),
        pivot + Vec3::new(0.0, 40.0, 0.0),
    );
}
