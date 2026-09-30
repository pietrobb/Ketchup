//! Push/Pull of a drawn shape on a plain (not program) part, through the same
//! app calls the Push/Pull tool and Enter make, checked on the exact solids.
use super::*;
use ketchup_model::exact_product::ExactBodyPackage;
use std::time::Duration;

const SIZE: [f64; 3] = [100.0, 60.0, 40.0];
const VOLUME: f64 = SIZE[0] * SIZE[1] * SIZE[2];

fn evaluate_exact(app: &mut KetchupApp) {
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker alongside the app tests");
    app.headless_force_exact_worker_path(&worker);
    let snapshot = app.document.current();
    let task = ketchup_application::evaluation::start_exact_evaluation(
        snapshot.clone(),
        &app.file.container_data,
        &app.exact.results,
        &app.exact.topology_results,
        Some(worker),
        || {},
    );
    let products = task.wait(Duration::from_secs(60)).unwrap();
    let report = ketchup_application::evaluation::publish_exact_products(
        &mut app.document,
        &mut app.exact.results,
        &mut app.exact.topology_results,
        &task,
        products,
    )
    .unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
    app.exact.source = Some(ketchup_application::evaluation::exact_source(&snapshot));
}

/// A rotation by `degrees` about z followed by a move to `at`.
fn placement(degrees: f64, at: [f64; 3]) -> Transform {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Transform::from_matrix([
        cos, -sin, 0.0, at[0], sin, cos, 0.0, at[1], 0.0, 0.0, 1.0, at[2], 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap()
}

/// One 100 x 60 x 40 block definition placed once per transform, so more
/// than one placement makes it a component with copies.
fn blocks(app: &mut KetchupApp, placements: &[Transform]) -> Vec<OccurrenceId> {
    let snapshot = app.document.current();
    let definition_id = DefinitionId(
        snapshot
            .definitions()
            .map(|item| item.id().0)
            .max()
            .unwrap_or(0)
            + 1,
    );
    let feature = snapshot
        .features()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let occurrence = snapshot
        .occurrences()
        .map(|item| item.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let mut commands = create_box_batch(
        definition_id,
        [FeatureId(feature), FeatureId(feature + 1)],
        OccurrenceId(occurrence),
        ["block", "block profile", "block extrusion", "block 1"],
        Vec3::ZERO,
        Vec3::new(SIZE[0], SIZE[1], SIZE[2]),
    )
    .commands()
    .to_vec();
    // Keep the definition and its features; place it once per transform, in
    // a document holding nothing else.
    commands.truncate(3);
    let clear = snapshot
        .occurrences()
        .map(|item| CanonicalCommand::DeleteOccurrence { id: item.id() })
        .chain(
            snapshot
                .definitions()
                .map(|item| CanonicalCommand::DeleteDefinition { id: item.id() }),
        );
    commands.splice(0..0, clear);
    let mut ids = Vec::new();
    for (index, transform) in placements.iter().enumerate() {
        let id = OccurrenceId(occurrence + index as u64);
        commands.push(CanonicalCommand::CreateOccurrence {
            id,
            definition_id,
            name: format!("block {}", index + 1),
            transform: *transform,
            parent: None,
            tag: None,
            visible: true,
        });
        ids.push(id);
    }
    app.document
        .apply_batch(&CommandBatch::new(commands))
        .unwrap();
    evaluate_exact(app);
    ids
}

/// Draws a closed shape in the plane through `origin` spanned by `x` and `y`
/// and leaves it selected, as the drawing tools do.
fn draw(
    app: &mut KetchupApp,
    origin: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
    segments: Vec<ProfileSegment>,
) -> SelectionId {
    let n = cross3(x, y);
    let transform = Transform::from_matrix([
        x[0], y[0], n[0], origin[0], x[1], y[1], n[1], origin[1], x[2], y[2], n[2], origin[2], 0.0,
        0.0, 0.0, 1.0,
    ])
    .unwrap();
    assert!(app.create_segment_profile_at(
        transform,
        segments,
        true,
        "model-default-box",
        "model-default-profile"
    ));
    app.selection.primary.clone().unwrap()
}

fn rectangle(low: [f64; 2], high: [f64; 2]) -> Vec<ProfileSegment> {
    let points = [low, [high[0], low[1]], high, [low[0], high[1]]];
    (0..4)
        .map(|index| ProfileSegment::Line {
            start_mm: points[index],
            end_mm: points[(index + 1) % 4],
        })
        .collect()
}

fn push_pull(app: &mut KetchupApp, distance: &str) -> bool {
    app.set_push_pull_distance_input(distance);
    assert!(app.start_preview(), "{}", app.digest);
    assert!(app.has_drawn_shape_preview(), "{}", app.digest);
    app.confirm_preview()
}

fn volume(app: &KetchupApp, occurrence: OccurrenceId) -> f64 {
    let snapshot = app.document.current();
    let definition_id = snapshot.occurrence(occurrence).unwrap().definition_id();
    let package = app
        .exact
        .topology_results
        .get_render(&snapshot, definition_id)
        .unwrap();
    let ExactBodyPackage::Graph(graph) = package.as_ref() else {
        panic!("an exact graph is required")
    };
    assert_eq!(graph.topology_counts[3..], [1, 1]);
    graph.volume_mm3
}

fn assert_volume(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-3 * expected,
        "{actual} != {expected}"
    );
}

#[test]
fn a_shape_on_a_side_of_a_rotated_component_mills_a_pocket_in_every_copy() {
    let mut app = KetchupApp::new();
    let turned = placement(30.0, [0.0, 0.0, 0.0]);
    let copies = blocks(&mut app, &[turned, placement(0.0, [300.0, 0.0, 0.0])]);
    let before = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    // The y- face of the turned copy: u along its x, v along z.
    let m = turned.matrix();
    draw(
        &mut app,
        [0.0, 0.0, 0.0],
        [m[0], m[4], m[8]],
        [0.0, 0.0, 1.0],
        rectangle([20.0, 10.0], [50.0, 30.0]),
    );

    assert!(push_pull(&mut app, "-8"), "{}", app.digest);
    // One step drew the shape, one used it up in the pocket.
    assert_eq!(app.undo_step_count(), undo_steps + 2);
    let snapshot = app.document.current();
    assert_eq!(
        snapshot.occurrences().count(),
        2,
        "the drawn shape is used up"
    );
    let definition_id = snapshot.occurrence(copies[0]).unwrap().definition_id();
    let definition = snapshot.definition(definition_id).unwrap();
    assert!(definition.feature_ids().iter().any(|id| matches!(
        snapshot.feature(*id).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Cut,
            ..
        }
    )));
    evaluate_exact(&mut app);
    for copy in copies {
        assert_volume(volume(&app, copy), VOLUME - 30.0 * 20.0 * 8.0);
    }

    app.undo();
    app.undo();
    assert_eq!(app.canonical_digest(), before);
}

#[test]
fn a_shape_pulled_out_of_a_plain_part_is_left_to_become_a_part_of_its_own() {
    let mut app = KetchupApp::new();
    let [block] = blocks(&mut app, &[placement(0.0, [10.0, 20.0, 0.0])])[..] else {
        unreachable!()
    };
    // A half disc of radius 20 on the top, bulging towards +y.
    let shape = draw(
        &mut app,
        [10.0, 20.0, 40.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        vec![
            ProfileSegment::Line {
                start_mm: [20.0, 30.0],
                end_mm: [60.0, 30.0],
            },
            ProfileSegment::CircularArc {
                start_mm: [60.0, 30.0],
                end_mm: [20.0, 30.0],
                center_mm: [40.0, 30.0],
                clockwise: false,
            },
        ],
    );
    assert!(app.drawn_shape_edit(&shape, 10.0).is_none());
    assert!(matches!(app.drawn_shape_edit(&shape, -10.0), Some(Ok(_))));
    app.set_push_pull_distance_input("10");
    assert!(app.start_preview(), "{}", app.digest);
    assert!(!app.has_drawn_shape_preview());
    assert_eq!(
        app.push_pull_preview_definition(),
        Some(shape.definition_id)
    );
    app.cancel_preview();
    evaluate_exact(&mut app);
    assert_volume(volume(&app, block), VOLUME);
}

#[test]
fn pushing_into_the_bottom_or_across_an_edge_mills_only_the_part() {
    let mut app = KetchupApp::new();
    let [block] = blocks(&mut app, &[placement(0.0, [0.0; 3])])[..] else {
        unreachable!()
    };
    // Drawn on the bottom looking up into the part: +5 goes into it.
    draw(
        &mut app,
        [0.0; 3],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        rectangle([10.0, 10.0], [30.0, 30.0]),
    );
    assert!(push_pull(&mut app, "5"), "{}", app.digest);
    evaluate_exact(&mut app);
    assert_volume(volume(&app, block), VOLUME - 20.0 * 20.0 * 5.0);

    // Half of this one lies past the end of the top face.
    draw(
        &mut app,
        [0.0, 0.0, 40.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        rectangle([80.0, 20.0], [130.0, 40.0]),
    );
    assert!(push_pull(&mut app, "-5"), "{}", app.digest);
    evaluate_exact(&mut app);
    assert_volume(volume(&app, block), VOLUME - 2000.0 - 20.0 * 20.0 * 5.0);
}

#[test]
fn a_shape_off_every_face_is_left_to_the_other_push_pull_paths() {
    let mut app = KetchupApp::new();
    blocks(&mut app, &[placement(0.0, [0.0; 3])]);
    let square = rectangle([10.0, 10.0], [30.0, 30.0]);
    let tilt = 0.5f64.sqrt();
    for (origin, y) in [
        // Floating above the part.
        ([0.0, 0.0, 100.0], [0.0, 1.0, 0.0]),
        // Through the top edge, tilted against the top.
        ([0.0, 0.0, 40.0], [0.0, tilt, tilt]),
        // In the top's plane but beside the part.
        ([150.0, 0.0, 40.0], [0.0, 1.0, 0.0]),
    ] {
        let shape = draw(&mut app, origin, [1.0, 0.0, 0.0], y, square.clone());
        assert!(
            app.drawn_shape_edit(&shape, -5.0).is_none(),
            "{origin:?} {y:?}"
        );
    }
}

#[test]
fn a_new_distance_replaces_the_preview_and_a_failed_one_clears_it() {
    let mut app = KetchupApp::new();
    let [block] = blocks(&mut app, &[placement(0.0, [0.0; 3])])[..] else {
        unreachable!()
    };
    draw(
        &mut app,
        [0.0, 0.0, 40.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        rectangle([10.0, 10.0], [30.0, 30.0]),
    );
    let before = app.canonical_digest();
    let steps = app.undo_step_count();

    app.set_push_pull_distance_input("-10");
    assert!(app.start_preview(), "{}", app.digest);
    assert!(app.has_drawn_shape_preview());
    app.set_push_pull_distance_input("-5");
    assert!(app.start_preview(), "{}", app.digest);
    assert!(app.has_drawn_shape_preview());
    // Typing the former distance back must not commit the superseded pocket.
    app.set_push_pull_distance_input("-10");
    assert!(!app.confirm_preview());
    assert_eq!(app.canonical_digest(), before);
    assert_eq!(app.undo_step_count(), steps);

    for failed in ["invalid", "0"] {
        app.set_push_pull_distance_input("-10");
        assert!(app.start_preview(), "{}", app.digest);
        app.set_push_pull_distance_input(failed);
        assert!(!app.start_preview(), "{failed}");
        assert!(
            !app.has_drawn_shape_preview(),
            "{failed} kept the old preview"
        );
        assert!(!app.confirm_preview(), "{failed}");
        assert_eq!(app.canonical_digest(), before);
        assert_eq!(app.undo_step_count(), steps);
    }

    assert!(push_pull(&mut app, "-10"), "{}", app.digest);
    assert_eq!(app.undo_step_count(), steps + 1);
    evaluate_exact(&mut app);
    assert_volume(volume(&app, block), VOLUME - 20.0 * 20.0 * 10.0);
}
