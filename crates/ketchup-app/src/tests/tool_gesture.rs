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
        pinned_way: None,
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
        if tool == "Move vector" {
            assert_refused_then_applied_to_the_second_box_alone(&mut app, tool);
            continue;
        }
        // The ended drag took its way with it: a distance has none to go.
        let digest = app.canonical_digest();
        assert!(!app.apply_value_input(), "{tool}: {}", app.digest);
        assert_eq!(app.digest, app.catalog.text("error-preview-stale"));
        assert!(no_gesture(&app), "{tool}");
        assert!(!app.apply_value_input(), "{tool} again: {}", app.digest);
        assert_eq!(
            app.digest,
            app.catalog.text("digest-move-direction-missing")
        );
        assert_eq!(app.canonical_digest(), digest, "{tool}");
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

/// The first box selected in Move, `axis` pinned, and a drag standing
/// `shown_mm` away the way the pointer points.
fn move_dragging(axis: Option<Axis>, shown_mm: Vec3) -> (KetchupApp, Vec3) {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let (origin, _) = app.occurrence_box_geometry(1).unwrap();
    app.dispatch_command(AppCommand::Move);
    app.set_move_axis_lock(axis);
    let mut drag = move_drag(&app, shown_mm);
    drag.axis = axis;
    app.set_move_session(ToolSessionPhase::Gesture, drag);
    (app, origin)
}

fn travel(app: &KetchupApp, origin: Vec3) -> Vec3 {
    app.occurrence_box_geometry(1).unwrap().0 - origin
}

/// The preview above the Move field naming `delta_mm` before Enter: how far
/// and which way along a pinned `axis`, the vector without one.
fn move_preview(app: &KetchupApp, axis: Option<Axis>, delta_mm: Vec3) -> Option<String> {
    let Some(axis) = axis else {
        return vector_preview(app, delta_mm);
    };
    let (along, name) = match axis {
        Axis::X => (delta_mm.x, "X"),
        Axis::Y => (delta_mm.y, "Y"),
        Axis::Z => (delta_mm.z, "Z"),
    };
    let sign = if along < 0.0 { '−' } else { '+' };
    Some(app.catalog.format(
        "value-move-axis-preview",
        &BTreeMap::from([
            ("value", format_height(along.abs())),
            ("direction", format!("{sign}{name}")),
        ]),
    ))
}

/// The preview above the Move field naming the vector `delta_mm` before Enter.
fn vector_preview(app: &KetchupApp, delta_mm: Vec3) -> Option<String> {
    let value = [delta_mm.x, delta_mm.y, delta_mm.z]
        .map(|value| format_height(value + 0.0))
        .join("; ");
    Some(
        app.catalog
            .format("value-length-preview", &BTreeMap::from([("value", value)])),
    )
}

/// Operator report 2026-11-03: with the axis pinned by an arrow key, a typed
/// distance went along +X/+Y/+Z whichever way the pointer showed, so 5000
/// jumped one way or the other depending on the side of the drag. A positive
/// distance now goes the way the pointer points, a negative one against it,
/// pinned or not, and the field shows before Enter how far and which way
/// along the pinned axis, or the resulting vector.
#[test]
fn a_typed_move_distance_goes_the_way_the_pointer_points() {
    for (axis, shown_mm, input, expected) in [
        (
            Some(Axis::X),
            Vec3::new(-40.0, 0.0, 0.0),
            "5000",
            Vec3::new(-5000.0, 0.0, 0.0),
        ),
        (
            Some(Axis::X),
            Vec3::new(-40.0, 0.0, 0.0),
            "-5000",
            Vec3::new(5000.0, 0.0, 0.0),
        ),
        (
            Some(Axis::X),
            Vec3::new(40.0, 0.0, 0.0),
            "5000",
            Vec3::new(5000.0, 0.0, 0.0),
        ),
        (
            Some(Axis::Z),
            Vec3::new(0.0, 0.0, -30.0),
            "8000",
            Vec3::new(0.0, 0.0, -8000.0),
        ),
        (
            Some(Axis::Y),
            Vec3::new(0.0, -12.0, 0.0),
            "-250",
            Vec3::new(0.0, 250.0, 0.0),
        ),
        (
            None,
            Vec3::new(-40.0, 0.0, 0.0),
            "5000",
            Vec3::new(-5000.0, 0.0, 0.0),
        ),
        (
            None,
            Vec3::new(-40.0, 0.0, 0.0),
            "-5000",
            Vec3::new(5000.0, 0.0, 0.0),
        ),
        (
            None,
            Vec3::new(0.0, 0.0, -30.0),
            "8000",
            Vec3::new(0.0, 0.0, -8000.0),
        ),
    ] {
        let case = format!("{axis:?} toward {shown_mm:?} typed {input}");
        let (mut app, origin) = move_dragging(axis, shown_mm);
        app.value_box.input = input.to_owned();
        assert_eq!(
            app.value_box_length_preview(),
            move_preview(&app, axis, expected),
            "{case}"
        );
        assert!(app.apply_value_input(), "{case}: {}", app.digest);
        assert!(
            length(travel(&app, origin) - expected) < 1e-6,
            "{case} travelled {:?}",
            travel(&app, origin)
        );
    }
}

/// A number typed after a finished drag corrects that drag along the way it
/// went, pinned or not: a drag toward -X corrected to 5000 ends 5000 mm
/// toward -X as one step, not 5000 mm toward +X on top of the drag.
#[test]
fn a_typed_distance_after_a_drag_corrects_it_the_way_it_went() {
    for axis in [Some(Axis::X), None] {
        for (input, expected) in [
            ("5000", Vec3::new(-5000.0, 0.0, 0.0)),
            ("-5000", Vec3::new(5000.0, 0.0, 0.0)),
        ] {
            let case = format!("{axis:?} typed {input}");
            let (mut app, origin) = move_dragging(axis, Vec3::new(-40.0, 0.0, 0.0));
            let undo_steps = app.undo_step_count();
            let drag = app.take_move_session(None).unwrap();
            assert!(app.commit_move_drag(&drag), "{case}: {}", app.digest);
            app.value_box.input = input.to_owned();
            assert_eq!(
                app.value_box_length_preview(),
                move_preview(&app, axis, expected),
                "{case}"
            );
            assert!(app.apply_value_input(), "{case}: {}", app.digest);
            assert!(
                length(travel(&app, origin) - expected) < 1e-6,
                "{case} travelled {:?}",
                travel(&app, origin)
            );
            assert_eq!(app.undo_step_count(), undo_steps + 1, "{case}");
        }
    }
}

/// Review 2026-11-04 UI-144: an axis pinned in the middle of a drag, before
/// the pointer moved again, left the drag showing no way, so a typed 100 said
/// "Point the direction first" though the axis was pinned. It goes the way
/// the drag went along that axis, and along +axis when the drag went across it.
#[test]
fn an_axis_pinned_mid_drag_keeps_the_way_the_drag_went() {
    for (shown_mm, pin, expected) in [
        (
            Vec3::new(-40.0, 15.0, 0.0),
            Axis::X,
            Vec3::new(-100.0, 0.0, 0.0),
        ),
        (
            Vec3::new(-40.0, 15.0, 0.0),
            Axis::Y,
            Vec3::new(0.0, 100.0, 0.0),
        ),
        (
            Vec3::new(-40.0, 15.0, 0.0),
            Axis::Z,
            Vec3::new(0.0, 0.0, 100.0),
        ),
    ] {
        let case = format!("drag {shown_mm:?} pinned {pin:?}");
        let (mut app, origin) = move_dragging(None, shown_mm);
        app.set_move_axis_lock(Some(pin));
        app.value_box.input = "100".to_owned();
        assert_eq!(
            app.value_box_length_preview(),
            move_preview(&app, Some(pin), expected),
            "{case}"
        );
        assert!(app.apply_value_input(), "{case}: {}", app.digest);
        assert!(
            length(travel(&app, origin) - expected) < 1e-6,
            "{case} travelled {:?}",
            travel(&app, origin)
        );
    }
    // Released again mid-drag, the drag shows no way until the pointer moves.
    let (mut app, _) = move_dragging(Some(Axis::X), Vec3::new(-40.0, 0.0, 0.0));
    app.set_move_axis_lock(None);
    app.value_box.input = "100".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(
        app.digest,
        app.catalog.text("digest-move-direction-missing")
    );
}

/// With no drag, no Move to correct and no pinned axis there is no way to
/// go: the distance moves nothing and says so, where it once went along +X.
#[test]
fn a_typed_move_distance_without_a_way_moves_nothing() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.dispatch_command(AppCommand::Move);
    let digest = app.canonical_digest();
    app.value_box.input = "5000".to_owned();
    assert!(!app.apply_value_input());
    assert_eq!(
        app.digest,
        app.catalog.text("digest-move-direction-missing")
    );
    assert_eq!(app.canonical_digest(), digest);
}

fn first_box_matrix(app: &KetchupApp) -> Vec<f64> {
    app.document
        .current()
        .occurrence(OccurrenceId(1))
        .unwrap()
        .transform()
        .matrix()
        .to_vec()
}

/// The rule Move follows holds for Rotate's angle: typed while the pointer
/// turns the part clockwise, or right after such a turn, 90 turns it 90°
/// clockwise, where it once turned counter-clockwise whatever the pointer
/// showed.
#[test]
fn a_typed_angle_turns_the_way_the_pointer_turned() {
    let mut expected = KetchupApp::new();
    expected.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    expected.dispatch_command(AppCommand::Rotate);
    expected.value_box.input = "-90".to_owned();
    assert!(expected.apply_value_input(), "{}", expected.digest);
    let clockwise = first_box_matrix(&expected);

    for finished in [false, true] {
        let mut app = KetchupApp::new();
        app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
        app.dispatch_command(AppCommand::Rotate);
        let mut drag = rotate_drag(&app, Axis::Z);
        drag.angle_degrees = -30.0;
        app.set_rotate_session(ToolSessionPhase::Gesture, drag);
        if finished {
            let drag = app.take_rotate_session(None).unwrap();
            assert!(app.commit_rotate_drag(&drag), "{}", app.digest);
        }
        app.value_box.input = "90".to_owned();
        assert!(
            app.apply_value_input(),
            "finished {finished}: {}",
            app.digest
        );
        let turned = first_box_matrix(&app);
        assert!(
            turned
                .iter()
                .zip(&clockwise)
                .all(|(a, b)| (a - b).abs() < 1e-9),
            "finished {finished}: {turned:?} is not 90° clockwise {clockwise:?}"
        );
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
