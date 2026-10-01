use super::*;
use ketchup_model::document::ProfileSegment;
use ketchup_model::topology::TopologicalElementKind;

const TABLE: &str = include_str!("../../../../examples/programs/table.star");
const APRON: &str = "\napron = board(\"table/apron-front\", (WIDTH - 2 * (INSET + LEG), 20, 80), at = (INSET + LEG, INSET + 25, HEIGHT - 100))\ndowels(legs[0], apron, dowel = \"8x30\", margin = 15)\n";

fn apply(source: &str, replace_document: bool) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.to_owned(),
        overrides: BTreeMap::new(),
        file_name: Some("table.star".to_owned()),
        replace_document,
    }
}

fn names(app: &KetchupApp) -> BTreeMap<String, u64> {
    app.document
        .current()
        .occurrences()
        .map(|occurrence| (occurrence.name().to_owned(), occurrence.id().0))
        .collect()
}

#[test]
fn ai_reads_and_edits_the_window_program_in_one_call_each() {
    let (mut app, mut bridge) = setup();
    let manual = app.live_bridge_stamp();

    assert_eq!(
        bridge.execute(&mut app, apply(TABLE, false), false),
        Err("program_rejected")
    );
    assert_eq!(
        take_error_details().unwrap()["details"]["program_code"],
        "not_program_document"
    );
    assert_eq!(app.live_bridge_stamp(), manual);
    let empty = bridge
        .execute(&mut app, Request::Program { expected: None }, false)
        .unwrap();
    assert!(empty["source"].is_null());

    let created = bridge.execute(&mut app, apply(TABLE, true), false).unwrap();
    assert_eq!(created["change"], "created", "{created}");
    assert_eq!(created["parts"], 5);
    assert_eq!(created["report"]["ok"], true);
    assert_eq!(created["report"]["relations_total"], 4, "{created}");
    assert_eq!(created["report"]["relations"][0]["kind"], "contact");
    assert_eq!(
        created["report"]["relations"][0]["faces"],
        serde_json::json!(["z-", "z+"])
    );
    let table = names(&app);
    let undo_steps = app.undo_step_count();

    let program = bridge
        .execute(&mut app, Request::Program { expected: None }, false)
        .unwrap();
    assert_eq!(program["source"], TABLE);
    let leg = program["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["name"] == "table/leg-back-left")
        .unwrap();
    assert_eq!(leg["occurrence_id"], table["table/leg-back-left"]);
    assert_eq!(leg["lines"], json!([[13, 13], [17, 17]]));

    let with_apron = format!("{TABLE}{APRON}");
    let added = bridge
        .execute(&mut app, apply(&with_apron, false), false)
        .unwrap();
    assert_eq!(added["change"], "incremental", "{added}");
    assert_eq!(added["added"][0]["name"], "table/apron-front");
    assert_eq!(added["undo_steps"], undo_steps + 1);
    let after = names(&app);
    assert!(table.iter().all(|(name, id)| after[name] == *id));
    assert_eq!(after.len(), 6);

    let unchanged = bridge
        .execute(&mut app, apply(&with_apron, false), false)
        .unwrap();
    assert_eq!(unchanged["change"], "unchanged");
    assert_eq!(unchanged["undo_steps"], undo_steps + 1);

    let before_error = app.live_bridge_stamp();
    assert_eq!(
        bridge.execute(
            &mut app,
            apply(&format!("{with_apron}board(\"broken\", (1, 2)\n"), false),
            false
        ),
        Err("program_rejected")
    );
    let details = take_error_details().unwrap();
    assert_eq!(details["details"]["published"], false);
    assert_eq!(details["target"], "program");
    assert!(
        details["reason"]
            .as_str()
            .unwrap()
            .contains("table.star:21:"),
        "{details}"
    );
    assert_eq!(app.live_bridge_stamp(), before_error);

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(names(&app), table);
    assert_eq!(app.document.current_rule_program().unwrap().source, TABLE);
}

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

/// Picks every face or edge of the part `name` in turn and returns what the
/// AI reads about each pick.
fn picks(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    name: &str,
    kind: TopologicalElementKind,
) -> Vec<Value> {
    let snapshot = app.document.current();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap();
    let package = app
        .exact
        .topology_results
        .get_render(&snapshot, occurrence.definition_id())
        .unwrap();
    let producer_feature_id = package.producer_feature_id();
    let count = package
        .topological_references()
        .iter()
        .filter(|reference| reference.kind == kind)
        .count() as u32;
    (0..count)
        .map(|ordinal| {
            assert!(app.select_topological_locator(
                ketchup_interaction::exact_projection::TopologicalPickLocator {
                    instance_path: InstancePath::root(occurrence.id()),
                    producer_feature_id,
                    kind,
                    ordinal,
                }
            ));
            let status = bridge.execute(app, Request::Status {}, false).unwrap();
            status["selected_context"]["program"].clone()
        })
        .collect()
}

#[test]
fn a_picked_face_or_edge_of_a_rotated_part_reads_in_program_terms() {
    const PARTS: &str = "rail = box(\"rail\", (400, 40, 20), at = (100, 200, 0))\n\
        rotate(rail, axis = (0, 1, 0), angle = -90)\n\
        top = extrude(\"top\", profile = [[\"front\", [0, 0], [100, 0]], [\"right\", [100, 0], [100, 50]], \
        [\"back\", [100, 50], [0, 50]], [\"left\", [0, 50], [0, 0]]], distance = 18, at = (600, 0, 0))\n";
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(PARTS, true), false).unwrap();
    evaluate_exact(&mut app);

    // Every face of the rotated box is named by its outward normal in the
    // box's own frame, whatever way it faces in the world.
    let faces = picks(&mut app, &mut bridge, "rail", TopologicalElementKind::Face);
    assert_eq!(faces.len(), 6);
    for face in &faces {
        assert_eq!(face["part"], "rail", "{face}");
        let normal = face["normal_local"].as_array().unwrap();
        let axis = (0..3)
            .find(|i| normal[*i].as_f64().unwrap().abs() > 0.999)
            .unwrap();
        let sign = if normal[axis].as_f64().unwrap() > 0.0 {
            "+"
        } else {
            "-"
        };
        let name = format!("{}{sign}", ["x", "y", "z"][axis]);
        assert_eq!(face["face"], name, "{face}");
        assert_eq!(face["on_face"], name, "{face}");
    }
    let up = faces
        .iter()
        .find(|face| face["normal_world"] == json!([0.0, 0.0, 1.0]))
        .expect("the rail has an upward face");
    assert_ne!(up["face"], "z+", "the rail is stood on its end: {up}");
    let up = up["on_face"].as_str().unwrap().to_owned();

    // A profile part's faces carry the program's segment and cap names; an
    // edge is the pair of faces fillet() takes.
    let faces = picks(&mut app, &mut bridge, "top", TopologicalElementKind::Face);
    let mut names = faces
        .iter()
        .map(|face| face["face"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["back", "end", "front", "left", "right", "start"]);
    let front = faces.iter().find(|face| face["face"] == "front").unwrap();
    assert_eq!(front["normal_world"], json!([0.0, -1.0, 0.0]), "{front}");
    assert_eq!(front["point_world_mm"][1], 0.0, "{front}");
    let edges = picks(&mut app, &mut bridge, "top", TopologicalElementKind::Edge);
    assert_eq!(edges.len(), 12);
    let edge = edges
        .iter()
        .map(|edge| edge["edge"].clone())
        .find(|edge| {
            let mut pair = serde_json::from_value::<Vec<String>>(edge.clone()).unwrap();
            pair.sort();
            pair == ["end", "front"]
        })
        .expect("the top has a front upper edge");

    // What was read goes straight back into the program.
    let used = format!(
        "{PARTS}c = box(\"c\", (20, 20, 20))\non(c, rail, face = \"{up}\")\n\
         fillet(top, edges = [{edge}], radius = 3)\n"
    );
    let (evaluated, report) = ketchup_program::run("table.star", &used, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}\n{used}"));
    assert!(report.ok, "{:#?}", report.issues);
    assert_eq!(evaluated.model.part("top").unwrap().finishes().count(), 1);
    let (c, rail) = (
        evaluated.model.part("c").unwrap(),
        evaluated.model.part("rail").unwrap(),
    );
    assert_eq!(
        c.world_bounds().0[2],
        rail.world_bounds().1[2],
        "c rests on the rail"
    );
}

fn face_count(app: &KetchupApp, name: &str) -> usize {
    let snapshot = app.document.current();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap();
    app.exact
        .topology_results
        .get_render(&snapshot, occurrence.definition_id())
        .unwrap()
        .topological_references()
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .count()
}

#[test]
fn a_fillet_on_a_picked_box_edge_is_written_into_the_program() {
    const BLOCK: &str =
        "block = box(\"block\", (100, 60, 40))\nrotate(block, axis = (0, 0, 1), angle = 30)\n";
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(BLOCK, true), false).unwrap();
    evaluate_exact(&mut app);
    assert_eq!(face_count(&app, "block"), 6);

    let edges = picks(&mut app, &mut bridge, "block", TopologicalElementKind::Edge);
    let ordinal = edges
        .iter()
        .position(|edge| {
            let mut pair = serde_json::from_value::<Vec<String>>(edge["edge"].clone()).unwrap();
            pair.sort();
            pair == ["x+", "y+"]
        })
        .expect("the block has an x+/y+ edge") as u32;
    let snapshot = app.document.current();
    let occurrence = snapshot.occurrences().next().unwrap();
    let locator = ketchup_interaction::exact_projection::TopologicalPickLocator {
        instance_path: InstancePath::root(occurrence.id()),
        producer_feature_id: app
            .exact
            .topology_results
            .get_render(&snapshot, occurrence.definition_id())
            .unwrap()
            .producer_feature_id(),
        kind: TopologicalElementKind::Edge,
        ordinal,
    };
    let undo_steps = app.undo_step_count();
    assert!(app.prepare_assistant_general_finish(
        locator,
        ketchup_application::topology::GeneralFinishKind::Fillet,
        5.0
    ));
    assert!(app.confirm_assistant_general_finish(), "{}", app.digest);

    // The program still owns the part and now says what was done by hand.
    let program = app.document.current_rule_program().unwrap().source.clone();
    assert!(program.starts_with(BLOCK), "{program}");
    let written = program[BLOCK.len()..].trim();
    assert!(
        written == "fillet(\"block\", edges=[[\"x+\", \"y+\"]], radius=5)"
            || written == "fillet(\"block\", edges=[[\"y+\", \"x+\"]], radius=5)",
        "{written}"
    );
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(names(&app)["block"], occurrence.id().0);
    evaluate_exact(&mut app);
    assert_eq!(face_count(&app, "block"), 7);

    // One Undo returns the sharp box and the source without the fillet.
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current_rule_program().unwrap().source, BLOCK);
}

#[test]
fn applied_program_answers_box_overlaps_with_the_exact_solids() {
    const TRIANGLE: &str =
        "t = extrude(\"t\", profile = [(0, 0), (100, 0), (0, 100)], distance = 20)\n";
    let mut wire = Wire::new();
    // Plain boards only: boxes decide everything, no exact check runs.
    let boards = wire.call(apply(TABLE, true)).result.unwrap();
    assert_eq!(boards["geometry_evaluated"], false, "{boards}");
    assert!(boards["exact_collisions"].is_null());

    // The cube sits in the corner the triangle leaves empty.
    let corner = format!("{TRIANGLE}c = box(\"c\", (20, 20, 20), at = (70, 70, 0))\n");
    let cleared = wire
        .call_within(apply(&corner, true), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(cleared["geometry_evaluated"], true, "{cleared}");
    assert_eq!(cleared["exact_collisions"]["cleared"], 1, "{cleared}");
    assert_eq!(cleared["report"]["ok"], true);
    assert_eq!(cleared["report"]["warnings"], 0, "{cleared}");
    // The relation map follows the exact solids: the cube's corner (70, 70)
    // is (140 - 100) / sqrt(2) mm from the hypotenuse.
    let relation = &cleared["report"]["relations"][0];
    assert_eq!(relation["kind"], "gap", "{cleared}");
    assert_eq!(relation["gap_mm"], 28.3, "{cleared}");

    // Moved onto the triangle's solid part, the same cube collides.
    let solid = format!("{TRIANGLE}c = box(\"c\", (20, 20, 20), at = (10, 10, 0))\n");
    let collides = wire
        .call_within(apply(&solid, false), Duration::from_secs(60))
        .result
        .unwrap();
    assert_eq!(collides["exact_collisions"]["collisions"], 1, "{collides}");
    assert_eq!(collides["report"]["ok"], false);
    assert_eq!(collides["report"]["issues"][0]["kind"], "collision");
    assert_eq!(
        collides["report"]["relations"][0]["status"], "collision",
        "{collides}"
    );
}

/// World origin and axes of the program part `name`.
fn part_frame(app: &KetchupApp, name: &str) -> ([f64; 3], [[f64; 3]; 3]) {
    let program = app.document.current_rule_program().unwrap().clone();
    let model = ketchup_application::plan_rule_program(&app.document, &program)
        .expect("the program plans")
        .evaluated
        .model;
    let part = model.parts.iter().find(|part| part.name == name).unwrap();
    (
        part.at_mm,
        [0, 1, 2].map(|axis| ketchup_program::frame::axis(&part.rotation, axis)),
    )
}

fn local_to_world(origin: [f64; 3], axes: &[[f64; 3]; 3], local: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| origin[i] + (0..3).map(|axis| axes[axis][i] * local[axis]).sum::<f64>())
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
    let n = ketchup_geometry::linalg::cross(x, y);
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

fn lines(points: &[[f64; 2]]) -> Vec<ProfileSegment> {
    (0..points.len())
        .map(|index| ProfileSegment::Line {
            start_mm: points[index],
            end_mm: points[(index + 1) % points.len()],
        })
        .collect()
}

/// Push/Pull of the selected drawn shape by `distance`, confirmed with Enter.
fn push_pull_drawn(app: &mut KetchupApp, distance: &str) -> bool {
    app.set_push_pull_distance_input(distance);
    assert!(app.start_preview(), "{}", app.digest);
    assert!(app.has_drawn_shape_preview(), "{}", app.digest);
    app.confirm_preview()
}

fn exact_volume(app: &KetchupApp, name: &str) -> f64 {
    let snapshot = app.document.current();
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap();
    let package = app
        .exact
        .topology_results
        .get_render(&snapshot, occurrence.definition_id())
        .unwrap();
    let ketchup_model::exact_product::ExactBodyPackage::Graph(graph) = package.as_ref() else {
        panic!("{name}: an exact graph is required")
    };
    assert_eq!(graph.topology_counts[3..], [1, 1], "{name}");
    graph.volume_mm3
}

fn assert_volume(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-3 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

const BLOCK_VOLUME: f64 = 100.0 * 60.0 * 40.0;

#[test]
fn a_shape_drawn_on_a_side_of_a_rotated_program_part_is_pushed_in_as_a_pocket() {
    const BLOCK: &str =
        "block = box(\"block\", (100, 60, 40))\nrotate(block, axis = (0, 0, 1), angle = 30)\n";
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(BLOCK, true), false).unwrap();
    let (origin, axes) = part_frame(&app, "block");
    // On the y- face (u along x, v along z), drawn looking out of it.
    draw(
        &mut app,
        local_to_world(origin, &axes, [0.0; 3]),
        axes[0],
        axes[2],
        lines(&[[20.0, 10.0], [50.0, 10.0], [50.0, 30.0], [20.0, 30.0]]),
    );
    let drawn = names(&app);
    assert_eq!(drawn.len(), 2);
    let undo_steps = app.undo_step_count();

    assert!(push_pull_drawn(&mut app, "-8"), "{}", app.digest);
    let program = app.document.current_rule_program().unwrap().source.clone();
    assert_eq!(
        &program[BLOCK.len()..],
        "pocket_shape(\"block\", \"y-\", [[20, 10], [50, 10], [50, 30], [20, 30]], 8, name=\"pocket 1\")\n"
    );
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    // The drawn shape is used up and the part keeps its identity.
    assert_eq!(
        names(&app),
        BTreeMap::from([("block".to_owned(), drawn["block"])])
    );
    evaluate_exact(&mut app);
    assert_volume(
        exact_volume(&app, "block"),
        BLOCK_VOLUME - 30.0 * 20.0 * 8.0,
    );

    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current_rule_program().unwrap().source, BLOCK);
    assert_eq!(names(&app), drawn);
}

#[test]
fn a_shape_drawn_facing_into_a_part_keeps_its_arc_and_pulls_out_a_boss() {
    const BLOCK: &str = "block = box(\"block\", (100, 60, 40))\n";
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(BLOCK, true), false).unwrap();
    // On the x+ face (u = y, v = z) with the drawing's normal pointing into
    // the part: drawn (a, b) is (u, v) = (a, 40 - b), so the loop is mirrored.
    // A quarter disc: the arc must stay the short way round in (u, v).
    draw(
        &mut app,
        [100.0, 0.0, 40.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0],
        vec![
            ProfileSegment::Line {
                start_mm: [30.0, 20.0],
                end_mm: [40.0, 20.0],
            },
            ProfileSegment::CircularArc {
                start_mm: [40.0, 20.0],
                end_mm: [30.0, 10.0],
                center_mm: [30.0, 20.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [30.0, 10.0],
                end_mm: [30.0, 20.0],
            },
        ],
    );
    // Against the drawing's normal is out of the part.
    assert!(push_pull_drawn(&mut app, "-10"), "{}", app.digest);
    let program = app.document.current_rule_program().unwrap().source.clone();
    assert_eq!(
        &program[BLOCK.len()..],
        "boss(\"block\", \"x+\", [[\"edge1\", [30, 20], [40, 20]], [\"edge2\", [40, 20], [30, 30], {\"center\": (30, 20), \"clockwise\": False}], [\"edge3\", [30, 30], [30, 20]]], 10, name=\"boss 1\")\n"
    );
    assert_eq!(names(&app).len(), 1);
    evaluate_exact(&mut app);
    assert_volume(
        exact_volume(&app, "block"),
        BLOCK_VOLUME + std::f64::consts::PI * 100.0 / 4.0 * 10.0,
    );
}

#[test]
fn only_shapes_lying_on_a_part_face_become_program_edits_and_may_run_off_it() {
    const BLOCK: &str = "block = box(\"block\", (100, 60, 40))\n";
    let (mut app, mut bridge) = setup();
    bridge.execute(&mut app, apply(BLOCK, true), false).unwrap();
    let square = lines(&[[10.0, 10.0], [30.0, 10.0], [30.0, 30.0], [10.0, 30.0]]);

    // Floating above the part, tilted against its top, or next to it on the
    // top's plane: none of these is on a face, so Push/Pull stays as it was.
    let tilt = 0.5f64.sqrt();
    for (origin, y) in [
        ([0.0, 0.0, 100.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, 40.0], [0.0, tilt, tilt]),
        ([150.0, 0.0, 40.0], [0.0, 1.0, 0.0]),
    ] {
        let shape = draw(&mut app, origin, [1.0, 0.0, 0.0], y, square.clone());
        assert!(
            app.drawn_shape_edit(&shape, -5.0).is_none(),
            "{origin:?} {y:?}"
        );
    }
    let program = app.document.current_rule_program().unwrap().source.clone();
    assert_eq!(program, BLOCK);

    // A preview is not confirmed once the distance changed under it.
    let shape = draw(
        &mut app,
        [0.0, 0.0, 40.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        lines(&[[80.0, 20.0], [130.0, 20.0], [130.0, 40.0], [80.0, 40.0]]),
    );
    app.set_push_pull_distance_input("-5");
    assert!(app.start_preview(), "{}", app.digest);
    app.set_push_pull_distance_input("-6");
    assert!(!app.confirm_preview());
    assert_eq!(app.document.current_rule_program().unwrap().source, BLOCK);

    // Running off the end of the top face mills only what lies on the part.
    assert_eq!(app.selection.primary.as_ref(), Some(&shape));
    assert!(push_pull_drawn(&mut app, "-5"), "{}", app.digest);
    let program = app.document.current_rule_program().unwrap().source.clone();
    assert_eq!(
        &program[BLOCK.len()..],
        "pocket_shape(\"block\", \"z+\", [[80, 20], [130, 20], [130, 40], [80, 40]], 5, name=\"pocket 1\")\n"
    );
    evaluate_exact(&mut app);
    assert_volume(
        exact_volume(&app, "block"),
        BLOCK_VOLUME - 20.0 * 20.0 * 5.0,
    );
}
