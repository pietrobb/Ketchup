//! Structural point 5 of review 2026-10-15: Move, Rotate, Scale, drawing and
//! Push/Pull share one gesture model. A value typed for a gesture the
//! document or selection left is refused the same way in every tool, Ctrl+Z
//! only ends a gesture that still stands, and every gesture takes a typed
//! value along every world axis.

use super::*;

const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

fn axis_index(axis: Axis) -> usize {
    match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    }
}

fn component(vector: Vec3, axis: usize) -> f64 {
    vector.to_array()[axis]
}

/// The initial box selected, plus a second box to drift the selection to.
fn two_boxes_first_selected() -> KetchupApp {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app
}

fn move_drag(app: &KetchupApp, delta_mm: Vec3) -> MoveDrag {
    let snapshot = app.document.current();
    MoveDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection: app.selected_move_reference().unwrap(),
        occurrence_paths: app.selected_instance_paths(),
        group_id: None,
        profile_target: None,
        pointer_start_world: Vec3::ZERO,
        plane_z: 0.0,
        axis: None,
        axis_reference: None,
        delta_mm,
        copy: false,
    }
}

fn rotate_drag(app: &KetchupApp, axis: Axis) -> RotateDrag {
    let snapshot = app.document.current();
    let occurrence_paths = app.selected_instance_paths();
    RotateDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection: app.selected_move_reference().unwrap(),
        centre_mm: app
            .rotation_centre_for(&|path| occurrence_paths.contains(path))
            .unwrap(),
        occurrence_paths,
        group_id: None,
        axis,
        reference_mm: None,
        angle_degrees: 0.0,
        copy: false,
    }
}

/// The world size of the initial box, whatever placement or feature shapes it.
fn world_size(app: &KetchupApp) -> Vec3 {
    let [minimum, maximum] = world_bounds(app, 1);
    maximum - minimum
}

fn scale_drag(app: &KetchupApp, axis: Axis) -> ScaleDrag {
    let snapshot = app.document.current();
    let occurrence_paths = app.selected_instance_paths();
    let size = world_size(app);
    let mut drives = [false; 3];
    drives[axis_index(axis)] = true;
    ScaleDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        selection: app.selected_move_reference().unwrap(),
        centre_mm: app
            .rotation_centre_for(&|path| occurrence_paths.contains(path))
            .unwrap(),
        occurrence_paths,
        group_id: None,
        centre_screen: Pos2::ZERO,
        reference_screen: Vec2::ZERO,
        along_reference: false,
        drives,
        factors: [1.0; 3],
        extent_mm: size.to_array(),
    }
}

fn box_face(definition_id: DefinitionId, occurrence: u64, axis: Axis, side: Side) -> SelectionId {
    SelectionId {
        definition_id,
        instance_path: InstancePath::root(OccurrenceId(occurrence)),
        element: ElementId::Face { axis, side },
    }
}

fn push_pull_drag(app: &KetchupApp, selection: SelectionId) -> PushPullDrag {
    let snapshot = app.document.current();
    PushPullDrag {
        source_document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest().to_owned(),
        selection,
        pointer_start: Pos2::ZERO,
        extent_start_mm: 20.0,
        screen_normal: Vec2::new(1.0, 0.0),
        pixels_per_mm: 1.0,
    }
}

fn no_gesture(app: &KetchupApp) -> bool {
    app.transform_tool.session.is_none()
        && app.gesture.drag.get::<PushPullDrag>().is_none()
        && app.gesture.drag.get::<PushPullAnchor>().is_none()
        && app.gesture.sketch.start.is_none()
}

fn world_bounds(app: &KetchupApp, occurrence: u64) -> [Vec3; 2] {
    let root = InstancePath::root(OccurrenceId(occurrence));
    app.rotation_bounds_for(&|path| *path == root).unwrap()
}

/// The value refused as typed for a gesture that no longer stands, the
/// gesture ended, and the same value confirmed again applied to the second
/// box alone as one new step.
fn assert_refused_then_applied_to_the_second_box_alone(app: &mut KetchupApp, tool: &str) {
    let first = world_bounds(app, 1);
    let second = world_bounds(app, 2);
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    assert!(!app.apply_value_input(), "{tool}: {}", app.digest);
    assert_eq!(
        app.digest,
        app.catalog.text("error-preview-stale"),
        "{tool}"
    );
    assert_eq!(app.canonical_digest(), digest, "{tool}");
    assert!(no_gesture(app), "{tool}");

    assert!(app.apply_value_input(), "{tool} again: {}", app.digest);
    assert_eq!(app.undo_step_count(), undo_steps + 1, "{tool}");
    assert_eq!(
        world_bounds(app, 1),
        first,
        "{tool} left the first box alone"
    );
    assert_ne!(
        world_bounds(app, 2),
        second,
        "{tool} changed the second box"
    );
}

/// One rule for every tool: a value typed for a gesture the selection moved
/// away from is refused and the gesture ends. Before, a Move distance moved
/// the newly selected part along world +x, ignoring the way the gesture
/// showed, and Push/Pull pushed the newly selected face, while a Move vector,
/// Rotate and Scale refused theirs.
#[test]
fn a_value_typed_for_a_gesture_the_selection_left_is_refused_by_every_tool() {
    for (tool, value) in [("Move distance", "150"), ("Move vector", "0;150;0")] {
        let mut app = two_boxes_first_selected();
        app.dispatch_command(AppCommand::Move);
        let drag = move_drag(&app, Vec3::new(0.0, 40.0, 0.0));
        app.set_move_session(ToolSessionPhase::Gesture, drag);
        app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
        app.value_box.input = value.to_owned();
        assert_refused_then_applied_to_the_second_box_alone(&mut app, tool);
    }

    let mut app = two_boxes_first_selected();
    app.dispatch_command(AppCommand::Rotate);
    let drag = rotate_drag(&app, Axis::Z);
    app.set_rotate_session(ToolSessionPhase::Gesture, drag);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    app.value_box.input = "90".to_owned();
    assert_refused_then_applied_to_the_second_box_alone(&mut app, "Rotate");

    let mut app = two_boxes_first_selected();
    app.dispatch_command(AppCommand::Scale);
    let drag = scale_drag(&app, Axis::X);
    app.set_scale_session(ToolSessionPhase::Gesture, drag);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    app.value_box.input = "2".to_owned();
    assert_refused_then_applied_to_the_second_box_alone(&mut app, "Scale");

    let mut app = two_boxes_first_selected();
    app.dispatch_command(AppCommand::PushPull);
    let first_top = box_face(INITIAL_BOX_DEFINITION, 1, Axis::Z, Side::Maximum);
    app.selection.select_exact(first_top.clone(), false);
    let drag = push_pull_drag(&app, first_top);
    app.gesture.drag.open(drag);
    app.selection
        .select_exact(box_face(DefinitionId(2), 2, Axis::Z, Side::Maximum), false);
    app.value_box.input = "10".to_owned();
    assert_refused_then_applied_to_the_second_box_alone(&mut app, "Push/Pull");
}

/// A shape whose points were placed on a document that changed since has
/// nothing left to take a typed value: it ends, and the value is refused.
#[test]
fn a_value_typed_for_a_shape_the_document_left_is_refused() {
    let mut app = KetchupApp::new();
    app.dispatch_command(AppCommand::Line);
    let snapshot = app.document.current();
    app.gesture.sketch.armed = true;
    app.gesture.sketch.start = Some(Vec3::new(0.0, 0.0, 20.0));
    app.gesture.sketch.source = Some((snapshot.document_id(), snapshot.revision_id()));
    assert!(app.create_box());
    let digest = app.canonical_digest();
    app.value_box.input = "100".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(app.digest, app.catalog.text("error-preview-stale"));
    assert_eq!(app.canonical_digest(), digest);
    assert!(no_gesture(&app));
}

/// Before, Ctrl+Z only dropped a Move, Rotate or Scale gesture the selection
/// had left, though it is no longer drawn, while the same Push/Pull gesture
/// let Ctrl+Z undo. A gesture that still stands is what Ctrl+Z ends first.
#[test]
fn ctrl_z_ends_a_standing_gesture_and_undoes_past_one_the_selection_left() {
    let mut app = two_boxes_first_selected();
    app.dispatch_command(AppCommand::Rotate);
    let drag = rotate_drag(&app, Axis::Z);
    app.set_rotate_session(ToolSessionPhase::Anchor, drag.clone());
    assert!(
        !app.undo(),
        "a standing gesture is ended, not a step undone"
    );
    assert!(app.rotate_session().is_none());
    assert_eq!(app.active_box_count(), 2);

    app.dispatch_command(AppCommand::Rotate);
    app.set_rotate_session(ToolSessionPhase::Anchor, drag);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    assert!(app.undo(), "{}", app.digest);
    assert_eq!(app.active_box_count(), 1, "the second box is undone");
    assert!(no_gesture(&app));
}

/// The tools review 2026-10-17 P3-G found outside [`GESTURES`], each with its
/// gesture standing on a fresh document.
fn standing_register_gestures() -> Vec<(&'static str, KetchupApp)> {
    let mut revolve = KetchupApp::new();
    assert!(revolve.create_closed_polyline(vec![
        [120.0, 0.0],
        [140.0, 0.0],
        [140.0, 30.0],
        [120.0, 30.0],
    ]));
    assert!(revolve.begin_revolve_tool());
    assert!(!revolve.add_revolve_axis_point(Vec3::new(110.0, 0.0, 0.0)));
    assert!(revolve.add_revolve_axis_point(Vec3::new(110.0, 30.0, 0.0)));
    assert!(revolve.has_revolve_preview());

    let mut offset = KetchupApp::new();
    assert!(offset.create_closed_polyline(vec![
        [120.0, 0.0],
        [220.0, 0.0],
        [220.0, 60.0],
        [120.0, 60.0],
    ]));
    offset.dispatch_command(AppCommand::PlanarOffset);
    offset.value_box.input = "5".to_owned();
    assert!(offset.refresh_planar_offset_preview());

    let mut finish = KetchupApp::new();
    install_initial_graph_result(&mut finish);
    select_initial_topological(&mut finish, TopologicalElementKind::Face, 3);
    finish.dispatch_command(AppCommand::Shell);
    assert!(finish.general_finish_preview_is_current());

    let mut sweep = KetchupApp::new();
    assert!(sweep.create_sweep_inputs(
        vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
        [0.0, 0.0],
        [0.0, 125.0],
    ));
    sweep.dispatch_command(AppCommand::Sweep);
    assert!(sweep.sweep_preview_is_current());

    let mut loft = KetchupApp::new();
    assert!(loft.create_loft_inputs(vec![
        (
            vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
            0.0,
        ),
        (
            vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
            80.0,
        ),
    ]));
    loft.dispatch_command(AppCommand::Loft);
    assert!(loft.loft_preview_is_current());

    let mut helix = KetchupApp::new();
    helix.dispatch_command(AppCommand::Helix);

    vec![
        ("Revolve", revolve),
        ("Offset", offset),
        ("Shell", finish),
        ("Sweep", sweep),
        ("Loft", loft),
        ("Helix", helix),
    ]
}

/// Review 2026-10-17 P3-G: Revolve, Offset, Shell/Fillet/Chamfer, Sweep, Loft
/// and Helix were listed by hand in Esc and Ctrl+Z and never checked for a
/// source that moved on. Through [`GESTURES`] each now answers like Move:
/// Ctrl+Z and Esc end a standing gesture without touching the document, and
/// one whose selection left holds no Ctrl+Z and refuses a typed value.
#[test]
fn every_registered_gesture_ends_on_ctrl_z_and_esc_and_drops_when_stale() {
    for (tool, mut app) in standing_register_gestures() {
        assert!(app.gesture_open(), "{tool} opens a gesture");
        assert!(app.gesture_holds_undo(), "{tool} holds Ctrl+Z");
        let digest = app.canonical_digest();
        let undo_steps = app.undo_step_count();
        assert!(!app.undo(), "{tool}: Ctrl+Z ends the gesture, not a step");
        assert!(!app.gesture_open(), "{tool} ended by Ctrl+Z");
        assert_eq!(app.canonical_digest(), digest, "{tool}");
        assert_eq!(app.undo_step_count(), undo_steps, "{tool}");
    }

    for (tool, mut app) in standing_register_gestures() {
        let digest = app.canonical_digest();
        app.cancel_or_deselect(false);
        assert!(!app.gesture_open(), "{tool} ended by Esc");
        assert_eq!(app.digest, app.catalog.text("digest-cancelled"), "{tool}");
        assert_eq!(app.canonical_digest(), digest, "{tool}");
    }

    for (tool, mut app) in standing_register_gestures() {
        app.selection.clear();
        if tool == "Helix" {
            // The Helix stands on no selection, so nothing can leave it.
            assert!(app.gesture_holds_undo(), "{tool}");
            continue;
        }
        assert!(app.gesture_open(), "{tool} is still drawn until a value");
        assert!(!app.gesture_holds_undo(), "{tool} stale holds no Ctrl+Z");
        let digest = app.canonical_digest();
        app.value_box.input = "5".to_owned();
        assert!(!app.apply_value_input(), "{tool}: {}", app.digest);
        assert_eq!(
            app.digest,
            app.catalog.text("error-preview-stale"),
            "{tool}"
        );
        assert!(!app.gesture_open(), "{tool} stale ended");
        assert_eq!(app.canonical_digest(), digest, "{tool}");
    }
}

/// A typed value finishes each gesture along whichever world axis it runs:
/// not only along +x, where the review found the tests stopped (UI-1).
#[test]
fn a_typed_value_finishes_every_gesture_along_every_world_axis() {
    let directions = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(1.0, -2.0, 2.0),
    ];
    for direction in directions {
        let mut app = KetchupApp::new();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        let (origin, _) = app.occurrence_box_geometry(1).unwrap();
        app.dispatch_command(AppCommand::Move);
        let drag = move_drag(&app, direction * 40.0);
        app.set_move_session(ToolSessionPhase::Gesture, drag);
        app.value_box.input = "150".to_owned();
        assert!(app.apply_value_input(), "{direction:?}: {}", app.digest);
        let travel = app.occurrence_box_geometry(1).unwrap().0 - origin;
        let expected = direction * (150.0 / length(direction));
        assert!(
            length(travel - expected) < 0.01,
            "Move toward {direction:?} travelled {travel:?}"
        );
    }

    for axis in AXES {
        let index = axis_index(axis);
        let mut app = KetchupApp::new();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.dispatch_command(AppCommand::Rotate);
        let drag = rotate_drag(&app, axis);
        app.set_rotate_session(ToolSessionPhase::Gesture, drag);
        app.value_box.input = "90".to_owned();
        assert!(
            app.apply_value_input(),
            "Rotate about {axis:?}: {}",
            app.digest
        );
        let matrix = app
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform()
            .matrix()
            .to_owned();
        for other in 0..3 {
            let expected = if other == index { 1.0 } else { 0.0 };
            assert!(
                (matrix[other * 4 + other] - expected).abs() < 1e-9,
                "Rotate 90° about {axis:?} keeps that axis and turns the others: {matrix:?}"
            );
        }
    }

    for axis in AXES {
        let index = axis_index(axis);
        let mut app = KetchupApp::new();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        let size = world_size(&app);
        app.dispatch_command(AppCommand::Scale);
        let drag = scale_drag(&app, axis);
        app.set_scale_session(ToolSessionPhase::Gesture, drag);
        app.value_box.input = "2".to_owned();
        assert!(
            app.apply_value_input(),
            "Scale along {axis:?}: {}",
            app.digest
        );
        let scaled = world_size(&app);
        for other in 0..3 {
            let factor = if other == index { 2.0 } else { 1.0 };
            assert!(
                (component(scaled, other) - component(size, other) * factor).abs() < 0.01,
                "Scale ×2 along {axis:?}: {size:?} became {scaled:?}"
            );
        }
    }

    for axis in AXES {
        for side in [Side::Minimum, Side::Maximum] {
            let index = axis_index(axis);
            let mut app = KetchupApp::new();
            let (origin, size) = app.occurrence_box_geometry(1).unwrap();
            app.dispatch_command(AppCommand::PushPull);
            let face = box_face(INITIAL_BOX_DEFINITION, 1, axis, side);
            app.selection.select_exact(face.clone(), false);
            let drag = push_pull_drag(&app, face);
            app.gesture.drag.open(drag);
            app.value_box.input = "10".to_owned();
            assert!(
                app.apply_value_input(),
                "Push/Pull {axis:?} {side:?}: {}",
                app.digest
            );
            let (pushed_origin, pushed_size) = app.occurrence_box_geometry(1).unwrap();
            let shift = if side == Side::Minimum { -10.0 } else { 0.0 };
            for other in 0..3 {
                let (grow, moved) = if other == index {
                    (10.0, shift)
                } else {
                    (0.0, 0.0)
                };
                assert!(
                    (component(pushed_size, other) - component(size, other) - grow).abs() < 0.01
                        && (component(pushed_origin, other) - component(origin, other) - moved)
                            .abs()
                            < 0.01,
                    "Push/Pull 10 mm on the {side:?} {axis:?} face: {origin:?} {size:?} \
                     became {pushed_origin:?} {pushed_size:?}"
                );
            }
        }
    }
}
