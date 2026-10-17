//! `draft_faces` through apply_and_verify: a draft gives the exact frustum in
//! one Undo step, a refused draft leaves the document, its saved bytes and Undo
//! untouched, and a draft of a shared component tilts every copy.

use super::drill_holes::{centroid, edit, face, saved};
use super::program_edit::{evaluate_exact, exact_volume};
use super::*;

/// The newest faces of the block: bottom, top, the two x sides and the two y sides.
struct Block {
    bottom: Value,
    top: Value,
    x_sides: [Value; 2],
    y_sides: [Value; 2],
}

impl Block {
    fn new(app: &mut KetchupApp, bridge: &mut LiveBridge) -> Self {
        let mut plane = |normal| face(app, bridge, "plane", normal);
        Self {
            bottom: plane([0.0, 0.0, -1.0]),
            top: plane([0.0, 0.0, 1.0]),
            x_sides: [plane([-1.0, 0.0, 0.0]), plane([1.0, 0.0, 0.0])],
            y_sides: [plane([0.0, -1.0, 0.0]), plane([0.0, 1.0, 0.0])],
        }
    }

    /// Width along x, depth along y and height of the undrafted block.
    fn size(&self) -> [f64; 3] {
        [
            centroid(&self.x_sides[1])[0] - centroid(&self.x_sides[0])[0],
            centroid(&self.y_sides[1])[1] - centroid(&self.y_sides[0])[1],
            centroid(&self.top)[2] - centroid(&self.bottom)[2],
        ]
    }

    fn sides(&self) -> Vec<&Value> {
        self.x_sides.iter().chain(&self.y_sides).collect()
    }
}

/// The volume of a width × depth × height block whose x sides are drafted by
/// `x_degrees` and y sides by `y_degrees` about its bottom: the section at
/// height z is (width − 2z·tan x)(depth − 2z·tan y).
fn drafted_volume([a, b, h]: [f64; 3], x_degrees: f64, y_degrees: f64) -> f64 {
    let (t, s) = (x_degrees.to_radians().tan(), y_degrees.to_radians().tan());
    a * b * h - (a * s + b * t) * h * h + 4.0 / 3.0 * t * s * h.powi(3)
}

fn draft(faces: &[&Value], neutral: &Value, angle_degrees: f64) -> Request {
    let reference = |face: &Value| face["reference_id"].as_str().unwrap().to_owned();
    edit(AssistantCadEditOperation::DraftFaces {
        definition_id: neutral["definition_id"].as_u64().unwrap(),
        name: "draft".to_owned(),
        target_feature_id: neutral["producer_feature_id"].as_u64().unwrap(),
        face_reference_ids: faces.iter().map(|face| reference(face)).collect(),
        neutral_face_reference_id: reference(neutral),
        angle_degrees,
    })
}

fn part_name(app: &KetchupApp, index: usize) -> String {
    let snapshot = app.document.current();
    snapshot.occurrences().nth(index).unwrap().name().to_owned()
}

#[test]
fn drafted_sides_give_the_exact_frustum_either_way_in_one_undo_step() {
    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let part = part_name(&app, 0);
    let block = Block::new(&mut app, &mut bridge);
    let size = block.size();
    assert!(
        (exact_volume(&app, &part) - drafted_volume(size, 0.0, 0.0)).abs() <= 1e-6,
        "the block is {size:?}"
    );
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));

    for angle in [5.0, -5.0] {
        let applied = bridge
            .execute(&mut app, draft(&block.sides(), &block.bottom, angle), false)
            .unwrap();
        assert_eq!(applied["published"], true, "{applied}");
        assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
        evaluate_exact(&mut app);
        let expected = drafted_volume(size, angle, angle);
        let volume = exact_volume(&app, &part);
        assert!(
            (volume - expected).abs() <= 1e-6 * expected,
            "{angle} degrees: {volume} != {expected}"
        );

        bridge
            .execute(&mut app, Request::Undo { expected: None }, false)
            .unwrap();
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(
            app.live_bridge_stamp().canonical_digest,
            stamp.canonical_digest
        );
        assert!(saved(&app) == bytes, "Undo must restore the saved bytes");
    }
}

#[test]
fn refused_drafts_name_their_case_and_leave_the_document_untouched() {
    let (mut app, mut bridge) = setup();
    // 50 mm high, so at 45 degrees the y sides meet below the top and the
    // draft closes the top to nothing.
    let taller = edit(AssistantCadEditOperation::SetDimension {
        feature_id: 2,
        constraint_id: None,
        value_mm: 50.0,
    });
    assert_eq!(
        bridge.execute(&mut app, taller, false).unwrap()["published"],
        true
    );
    evaluate_exact(&mut app);
    let block = Block::new(&mut app, &mut bridge);
    let [width, depth, height] = block.size();
    assert!(2.0 * height > width.min(depth), "{:?}", block.size());
    let mut forged = block.x_sides[0].clone();
    forged["reference_id"] = json!("f".repeat(64));
    let before = (
        app.live_bridge_stamp(),
        app.undo_step_count(),
        app.redo_step_count(),
        saved(&app),
    );

    for (request, code, case) in [
        (
            draft(&block.sides(), &block.bottom, 45.5),
            "invalid_program",
            "an angle from 0.1 to 45 degrees either way",
        ),
        (
            draft(&block.sides(), &block.bottom, 0.05),
            "invalid_program",
            "an angle from 0.1 to 45 degrees either way",
        ),
        (
            draft(&[&block.x_sides[0], &block.bottom], &block.bottom, 5.0),
            "invalid_program",
            "a neutral face not among them",
        ),
        (
            draft(&[&forged], &block.bottom, 5.0),
            "planning_rejected",
            "planning.cad_topology_reference_unavailable",
        ),
        (
            draft(&[&block.top], &block.bottom, 5.0),
            "exact_evaluation_rejected",
            "draft_face_perpendicular_to_pull",
        ),
        (
            draft(&block.sides(), &block.bottom, 45.0),
            "exact_evaluation_rejected",
            "draft_face_degenerates",
        ),
    ] {
        let result = bridge.execute(&mut app, request, false);
        let details = take_error_details().map(|details| details.to_string());
        assert_eq!(
            result.as_ref().err(),
            Some(&code),
            "{case}: {result:?} {details:?}"
        );
        let details = details.unwrap();
        assert!(details.contains(case), "{case}: {details}");
        let after = (
            app.live_bridge_stamp(),
            app.undo_step_count(),
            app.redo_step_count(),
            saved(&app),
        );
        assert!(after == before, "{case}: the refusal changed the document");
    }
}

/// A component works like SketchUp's: a draft of the shared definition tilts
/// every copy; after Make Unique a draft tilts only that copy.
#[test]
fn a_draft_tilts_every_copy_of_a_component_until_one_is_made_unique() {
    let (mut app, mut bridge) = setup();
    let source = app.document.current().occurrences().next().unwrap().id();
    let copied = edit(AssistantCadEditOperation::Copy {
        selector: ketchup_assistant::sidecar::AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![source.0],
        },
        translation_mm: [5000.0, 0.0, 0.0],
    });
    assert_eq!(
        bridge.execute(&mut app, copied, false).unwrap()["published"],
        true
    );
    let snapshot = app.document.current();
    let copy = snapshot.occurrences().nth(1).unwrap().id();
    let shared = snapshot.occurrence(source).unwrap().definition_id();
    assert_eq!(snapshot.occurrence(copy).unwrap().definition_id(), shared);
    let names = [part_name(&app, 0), part_name(&app, 1)];
    let volumes = |app: &KetchupApp| names.clone().map(|name| exact_volume(app, &name));
    evaluate_exact(&mut app);
    let block = Block::new(&mut app, &mut bridge);
    let size = block.size();
    let close = |volume: f64, expected: f64| (volume - expected).abs() <= 1e-6 * expected;

    let x_sides: Vec<_> = block.x_sides.iter().collect();
    let undo = app.undo_step_count();
    let shared_draft = draft(&x_sides, &block.bottom, 5.0);
    assert_eq!(
        bridge.execute(&mut app, shared_draft, false).unwrap()["published"],
        true
    );
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    evaluate_exact(&mut app);
    let x_drafted = drafted_volume(size, 5.0, 0.0);
    let [a, b] = volumes(&app);
    assert!(close(a, x_drafted) && close(b, x_drafted), "{a} {b}");

    let unique = edit(AssistantCadEditOperation::MakeOccurrenceUnique {
        occurrence_id: copy.0,
    });
    let undo = app.undo_step_count();
    assert_eq!(
        bridge.execute(&mut app, unique, false).unwrap()["published"],
        true
    );
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    let own = app
        .document
        .current()
        .occurrence(copy)
        .unwrap()
        .definition_id();
    assert_ne!(own, shared);
    evaluate_exact(&mut app);
    let shared_definition = |app: &KetchupApp| {
        let snapshot = app.document.current();
        format!("{:?}", snapshot.definition(shared).unwrap())
    };
    let before = shared_definition(&app);
    // The y sides of the unique copy still face along y after the x draft.
    let bottom = face(&mut app, &mut bridge, "plane", [0.0, 0.0, -1.0]);
    assert_eq!(bottom["definition_id"].as_u64(), Some(own.0));
    let y_sides = [[0.0, -1.0, 0.0], [0.0, 1.0, 0.0]]
        .map(|normal| face(&mut app, &mut bridge, "plane", normal));
    let undo = app.undo_step_count();
    let own_draft = bridge.execute(
        &mut app,
        draft(&[&y_sides[0], &y_sides[1]], &bottom, 3.0),
        false,
    );
    assert!(own_draft.is_ok(), "{:?}", take_error_details());
    assert_eq!(own_draft.unwrap()["published"], true);
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    evaluate_exact(&mut app);
    let [a, b] = volumes(&app);
    assert!(close(a, x_drafted), "the original changed: {a}");
    assert!(
        close(b, drafted_volume(size, 5.0, 3.0)),
        "the unique copy: {b}"
    );
    assert_eq!(
        shared_definition(&app),
        before,
        "the shared definition changed"
    );
}

#[test]
fn the_draft_tool_drafts_the_selected_faces_about_the_clicked_one_in_one_undo_step() {
    use eframe::egui::{Pos2, Rect, Vec2};
    use ketchup_interaction::Vec3;
    use ketchup_interaction::exact_projection::TopologicalPickLocator;
    use ketchup_model::topology::TopologicalElementKind;

    let (mut app, mut bridge) = setup();
    evaluate_exact(&mut app);
    let part = part_name(&app, 0);
    let snapshot = app.document.current();
    let root = snapshot.occurrences().next().unwrap().id();
    let world = snapshot.world_transform_for_occurrence(root).unwrap();
    drop(snapshot);
    let block = Block::new(&mut app, &mut bridge);
    let (stamp, undo, bytes) = (app.live_bridge_stamp(), app.undo_step_count(), saved(&app));

    // The same draft sent as MCP gives the volume the tool must give.
    let sent = bridge
        .execute(&mut app, draft(&block.sides(), &block.top, 5.0), false)
        .unwrap();
    assert_eq!(sent["published"], true, "{sent}");
    evaluate_exact(&mut app);
    let expected = exact_volume(&app, &part);
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(saved(&app) == bytes);

    let select_sides = |app: &mut KetchupApp| {
        for (index, side) in block.sides().into_iter().enumerate() {
            assert!(app.select_topological_locator_additive(
                TopologicalPickLocator {
                    instance_path: InstancePath::root(root),
                    producer_feature_id: FeatureId(side["producer_feature_id"].as_u64().unwrap()),
                    kind: TopologicalElementKind::Face,
                    ordinal: u32::try_from(side["ordinal"].as_u64().unwrap()).unwrap(),
                },
                index > 0,
            ));
        }
    };
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0));
    let [x, y, z] = centroid(&block.top);
    let [wx, wy, wz] = world.transform_point([x + 10.0, y, z]);
    let click = app.project(Vec3::new(wx, wy, wz), rect);
    select_sides(&mut app);
    app.dispatch_command(AppCommand::Draft);
    assert_eq!(app.active_tool, ActiveTool::Draft);
    assert_eq!(app.value_box.input, "5");
    // A Slovak decimal comma is the same angle (GX-15).
    app.value_box.decimal = crate::length_input::DecimalSeparator::Comma;
    app.value_box.input = "5,0".to_owned();
    assert!(app.draft_at_screen(click, rect), "{}", app.digest);
    app.finish_window_edit();
    assert_eq!(app.digest, app.catalog.text("digest-draft-created"));
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    evaluate_exact(&mut app);
    let volume = exact_volume(&app, &part);
    assert!(
        (volume - expected).abs() <= 1e-9 * expected,
        "{volume} != {expected}"
    );
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(saved(&app) == bytes, "Undo must restore the saved bytes");

    // A refused angle, a typo and no selected face change nothing.
    let unchanged = |app: &KetchupApp| {
        app.live_bridge_stamp().canonical_digest == stamp.canonical_digest
            && app.undo_step_count() == undo
            && saved(app) == bytes
    };
    select_sides(&mut app);
    app.value_box.input = "60".to_owned();
    assert!(app.draft_at_screen(click, rect));
    app.finish_window_edit();
    assert!(
        app.digest
            .starts_with("invalid_program: assistant CAD draft is invalid"),
        "{}",
        app.digest
    );
    assert!(unchanged(&app), "{}", app.digest);
    app.value_box.input = "five".to_owned();
    assert!(!app.draft_at_screen(click, rect));
    assert_eq!(app.digest, app.catalog.text("digest-draft-invalid-angle"));
    app.value_box.input = "5".to_owned();
    app.selection.topological.clear();
    assert!(!app.draft_at_screen(click, rect));
    assert_eq!(app.digest, app.catalog.text("digest-draft-faces-required"));
    assert!(!app.live.edits.is_running());
    assert!(unchanged(&app));
}
