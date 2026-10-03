use super::super::tests::setup;
use super::*;
use ketchup_application::evaluation::{publish_exact_products, start_exact_evaluation};
use ketchup_model::document::RuleProgramSource;

#[path = "../../../ketchup-model/tests/support/integration_support.rs"]
pub(crate) mod integration_support;

fn evaluate_exact(app: &mut KetchupApp) {
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build current ketchup-exact-worker alongside the app tests");
    app.headless_force_exact_worker_path(&worker);
    let task = start_exact_evaluation(
        app.document.current(),
        &app.file.container_data,
        &app.exact.results,
        &app.exact.topology_results,
        Some(worker),
        || {},
    );
    let products = task.wait(Duration::from_secs(60)).unwrap();
    let evidence = publish_exact_products(
        &mut app.document,
        &mut app.exact.results,
        &mut app.exact.topology_results,
        &task,
        products,
    )
    .unwrap();
    assert!(
        evidence.complete && evidence.topology_complete,
        "{evidence:?}"
    );
}

fn measure(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    faces: [FaceTarget; 2],
    mode: MeasurementMode,
    direction: Option<[f64; 3]>,
) -> Response {
    let context = egui::Context::default();
    let (reply, response) = mpsc::sync_channel(1);
    bridge.start_measurement(
        app,
        &context,
        1,
        reply,
        Arc::new(AtomicBool::new(false)),
        app.live_bridge_stamp(),
        faces,
        mode,
        direction,
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        bridge.poll_program_check_job(app, &context);
        if let Ok(response) = response.try_recv() {
            assert!(bridge.program_check_job.is_none());
            return response;
        }
        assert!(Instant::now() < deadline, "face measurement did not finish");
        std::thread::yield_now();
    }
}

// Read both the face row and the explicit member path through the public queries.
// None selects the cylindrical wall of the drilled fixture, never a native ordinal.
fn face_target(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    part: &str,
    normal: Option<[f64; 3]>,
) -> FaceTarget {
    let program = bridge
        .execute(app, Request::Program { expected: None }, false)
        .unwrap();
    let part = program["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == part)
        .unwrap();
    let page = bridge
        .execute(
            app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(json!({"kind":"faces","limit":100})).unwrap(),
            },
            false,
        )
        .unwrap();
    let mut rows = page["items"].as_array().unwrap().clone();
    let mut cursor = page["next_cursor"].clone();
    while !cursor.is_null() {
        let page = bridge
            .execute(
                app,
                Request::Query {
                    expected: None,
                    query: serde_json::from_value(
                        json!({"kind":"faces","limit":100,"cursor":cursor}),
                    )
                    .unwrap(),
                },
                false,
            )
            .unwrap();
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        cursor = page["next_cursor"].clone();
    }
    let row = rows
        .iter()
        .find(|row| {
            row["definition_id"] == part["definition_id"]
                && match normal {
                    Some(normal) => {
                        row["geometry"]["surface_kind"] == "plane"
                            && (0..3).all(|axis| {
                                row["geometry"]["unit_normal"][axis]
                                    .as_f64()
                                    .is_some_and(|n| (n - normal[axis]).abs() < 1e-7)
                            })
                    }
                    None => row["geometry"]["surface_kind"] == "cylinder",
                }
        })
        .unwrap_or_else(|| {
            panic!("native face missing: part={part:?}, normal={normal:?}, rows={rows:?}")
        });
    let canonical: ketchup_model::document::InstancePath =
        serde_json::from_value(part["instance_path"].clone()).unwrap();
    let instances = bridge
        .execute(
            app,
            Request::Query {
                expected: None,
                query: serde_json::from_value(
                    json!({"kind":"instances","limit":100,"definition_id":part["definition_id"]}),
                )
                .unwrap(),
            },
            false,
        )
        .unwrap();
    assert!(
        instances["next_cursor"].is_null(),
        "small measurement fixture must fit one instance page"
    );
    let instance = instances["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| {
            let path = &row["instance_path"];
            path["root_occurrence_id"] == canonical.root_occurrence().0
                && path["steps"].as_array().unwrap().len() == canonical.steps().len()
                && path["steps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(canonical.steps())
                    .all(|(wire, step)| match step {
                        ketchup_model::document::InstancePathStep::Group(id) => {
                            wire["kind"] == "group" && wire["local_id"] == id.0
                        }
                        ketchup_model::document::InstancePathStep::Occurrence(id) => {
                            wire["kind"] == "occurrence" && wire["local_id"] == id.0
                        }
                    })
        })
        .expect("public instance query must expose the selected path");
    FaceTarget {
        entity_id: row["id"].as_u64().unwrap(),
        instance_path: serde_json::from_value(instance["instance_path"].clone()).unwrap(),
    }
}

const DRILLED: &str = "bottom=box('bottom',(100,100,18))\n\
    hole(bottom,'z+',at=(50,50),diameter=8,depth=12)\n\
    shelf=box('shelf',(100,100,18),at=(0,0,668))";

fn fixture(source: &str) -> (KetchupApp, LiveBridge) {
    let (mut app, mut bridge) = setup();
    app.new_document();
    let program = RuleProgramSource {
        file_name: "faces.star".into(),
        source: source.into(),
        overrides: BTreeMap::new(),
    };
    app.apply_program_source(program.clone(), false).unwrap();
    // Keep real work on both sides of the history cursor, not just empty counters.
    let mut future = program;
    future
        .source
        .push_str("\nbox('redo-only',(1,1,1),at=(1000,0,0))");
    app.apply_program_source(future, false).unwrap();
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    evaluate_exact(&mut app);
    let selected = app.document.current().occurrences().next().unwrap().id();
    app.selection.select_occurrence(selected, false);
    assert!(app.undo_step_count() > 0 && app.redo_step_count() > 0);
    (app, bridge)
}

fn reject_without_changes(
    app: &mut KetchupApp,
    bridge: &mut LiveBridge,
    faces: [FaceTarget; 2],
    mode: MeasurementMode,
    direction: Option<[f64; 3]>,
) {
    let snapshot = app.document.current();
    let source = app.document.current_rule_program().cloned();
    let stamp = app.live_bridge_stamp();
    let history = (app.undo_step_count(), app.redo_step_count(), app.is_dirty());
    let selection = LiveBridge::selection(app);
    let paths = app.selected_instance_paths();
    assert!(!selection.as_ref().unwrap().is_empty());
    let response = measure(app, bridge, faces, mode, direction);
    assert!(
        !response.ok,
        "must not invent an exact measurement: {response:?}"
    );
    assert!(
        response
            .result
            .as_ref()
            .is_some_and(|value| value["distance_mm"].is_null() && value["state"] != "verified"),
        "failed query exposed a measurement: {response:?}"
    );
    assert_eq!(app.document.current().scene_query(), snapshot.scene_query());
    assert_eq!(app.document.current_rule_program(), source.as_ref());
    assert_eq!(app.live_bridge_stamp(), stamp);
    assert_eq!(
        (app.undo_step_count(), app.redo_step_count(), app.is_dirty()),
        history
    );
    assert_eq!(LiveBridge::selection(app), selection);
    assert_eq!(app.selected_instance_paths(), paths);
}

fn assert_distance(response: Response, expected: f64, method: &str) -> Value {
    assert!(response.ok, "{response:?}");
    let value = response.result.unwrap();
    assert_eq!(value["state"], "verified", "{value}");
    assert!(
        (value["distance_mm"].as_f64().unwrap() - expected).abs() < 1e-6,
        "{value}"
    );
    assert_eq!(value["unit"], "mm");
    assert_eq!(value["method"], method);
    value
}

#[test]
fn public_face_measurement_proves_drilled_650_mm_clearance_and_is_read_only() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = fixture(DRILLED);
    let snapshot = app.document.current();
    let source = app.document.current_rule_program().cloned();
    let stamp = app.live_bridge_stamp();
    let faces = [
        face_target(&mut app, &mut bridge, "bottom", Some([0.0, 0.0, 1.0])),
        face_target(&mut app, &mut bridge, "shelf", Some([0.0, 0.0, -1.0])),
    ];
    let history = (app.undo_step_count(), app.redo_step_count(), app.is_dirty());
    let selection = LiveBridge::selection(&app);
    for mode in [MeasurementMode::Minimum, MeasurementMode::SupportingPlanes] {
        let supporting = matches!(mode, MeasurementMode::SupportingPlanes);
        assert_distance(
            measure(
                &mut app,
                &mut bridge,
                faces.clone(),
                mode,
                supporting.then_some([0.0, 0.0, 1.0]),
            ),
            650.0,
            if supporting {
                "exact_supporting_planes"
            } else {
                "occt_trimmed_face_distance"
            },
        );
    }
    assert_eq!(app.document.current().scene_query(), snapshot.scene_query());
    assert_eq!(app.document.current_rule_program(), source.as_ref());
    assert_eq!(app.live_bridge_stamp(), stamp);
    assert_eq!(
        (app.undo_step_count(), app.redo_step_count(), app.is_dirty()),
        history
    );
    assert_eq!(LiveBridge::selection(&app), selection);
    let mut invalid = faces.clone();
    invalid[0].entity_id = u64::MAX;
    reject_without_changes(
        &mut app,
        &mut bridge,
        invalid,
        MeasurementMode::Minimum,
        None,
    );
    app.new_document();
    assert!(
        bridge
            .measure_now(
                &app,
                stamp,
                faces,
                MeasurementMode::Minimum,
                None,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
    );
    assert!(app.document.current().occurrences().next().is_none());
    assert_eq!(app.undo_step_count(), 0);
}

#[test]
fn nested_shared_face_measurement_uses_world_placement_and_finite_faces() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = fixture(
        "a=box('a',(20,30,40),at=(10,20,0))\n\
         g=group('inner',[a])\n\
         c=component('assembly',[g])\n\
         instance('second',c,at=(100,100,80),x=(0,1,0))",
    );
    let a = face_target(&mut app, &mut bridge, "a", Some([0.0, 0.0, 1.0]));
    let b = face_target(&mut app, &mut bridge, "second/a", Some([0.0, 0.0, 1.0]));
    assert_eq!(a.entity_id, b.entity_id, "one shared definition face");
    assert_ne!(a.instance_path, b.instance_path);
    assert!(a.instance_path.steps.len() >= 2 && b.instance_path.steps.len() >= 2);
    let scene = app.document.current().scene_query();
    let source = app.document.current_rule_program().cloned();
    let history = (app.undo_step_count(), app.redo_step_count(), app.is_dirty());
    let selection = LiveBridge::selection(&app);
    // Original top: x=10..30,y=20..50,z=40. Rotated instance top:
    // x=50..80,y=110..130,z=120. Finite-face gaps are (20,60,80),
    // not the 80 mm supporting-plane gap or the 40 mm solid Z gap.
    let faces = [a, b];
    let value = assert_distance(
        measure(
            &mut app,
            &mut bridge,
            faces.clone(),
            MeasurementMode::Minimum,
            None,
        ),
        10400.0_f64.sqrt(),
        "occt_trimmed_face_distance",
    );
    assert_eq!(value["targets"], json!(faces));
    let value = assert_distance(
        measure(
            &mut app,
            &mut bridge,
            faces.clone(),
            MeasurementMode::SupportingPlanes,
            Some([0.0, 0.0, 2.0]),
        ),
        80.0,
        "exact_supporting_planes",
    );
    assert_eq!(value["finite_face_minimum"], false);
    assert_eq!(value["direction_world"], json!([0.0, 0.0, 1.0]));
    assert_distance(
        measure(
            &mut app,
            &mut bridge,
            [faces[1].clone(), faces[0].clone()],
            MeasurementMode::SupportingPlanes,
            Some([0.0, 0.0, 1.0]),
        ),
        -80.0,
        "exact_supporting_planes",
    );
    assert_eq!(app.document.current().scene_query(), scene);
    assert_eq!(app.document.current_rule_program(), source.as_ref());
    assert_eq!(
        (app.undo_step_count(), app.redo_step_count(), app.is_dirty()),
        history
    );
    assert_eq!(LiveBridge::selection(&app), selection);
}

#[test]
fn absent_exact_face_evidence_never_falls_back_to_mesh_or_bounds() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = fixture(DRILLED);
    let faces = [
        face_target(&mut app, &mut bridge, "bottom", Some([0.0, 0.0, 1.0])),
        face_target(&mut app, &mut bridge, "shelf", Some([0.0, 0.0, -1.0])),
    ];
    // Retain the scene and render products, but remove the selected topology's
    // exact evidence. Even supporting-plane mode must not infer from the AABB.
    assert!(!app.exact.results.is_empty());
    app.exact.topology_results.clear();
    for mode in [MeasurementMode::Minimum, MeasurementMode::SupportingPlanes] {
        let direction =
            matches!(mode, MeasurementMode::SupportingPlanes).then_some([0.0, 0.0, 1.0]);
        reject_without_changes(&mut app, &mut bridge, faces.clone(), mode, direction);
    }
    evaluate_exact(&mut app);
    let faces = [
        face_target(&mut app, &mut bridge, "bottom", Some([0.0, 0.0, 1.0])),
        face_target(&mut app, &mut bridge, "shelf", Some([0.0, 0.0, -1.0])),
    ];
    assert_distance(
        measure(&mut app, &mut bridge, faces, MeasurementMode::Minimum, None),
        650.0,
        "occt_trimmed_face_distance",
    );
}

#[test]
fn invalid_face_references_preserve_source_selection_and_usable_undo_redo() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = fixture(DRILLED);
    let faces = [
        face_target(&mut app, &mut bridge, "bottom", Some([0.0, 0.0, 1.0])),
        face_target(&mut app, &mut bridge, "shelf", Some([0.0, 0.0, -1.0])),
    ];
    let mut missing_face = faces.clone();
    missing_face[0].entity_id = u64::MAX;
    let mut missing_instance = faces.clone();
    missing_instance[0].instance_path.root_occurrence_id = u64::MAX;
    let mut wrong_definition = faces.clone();
    wrong_definition[0].instance_path = faces[1].instance_path.clone();
    for invalid in [missing_face, missing_instance, wrong_definition] {
        reject_without_changes(
            &mut app,
            &mut bridge,
            invalid,
            MeasurementMode::Minimum,
            None,
        );
    }
    assert_distance(
        measure(&mut app, &mut bridge, faces, MeasurementMode::Minimum, None),
        650.0,
        "occt_trimmed_face_distance",
    );
    let scene = app.document.current().scene_query();
    let source = app.document.current_rule_program().cloned();
    bridge
        .execute(&mut app, Request::Undo { expected: None }, false)
        .unwrap();
    assert!(app.document.current().occurrences().next().is_none());
    assert!(app.document.current_rule_program().is_none());
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert_eq!(app.document.current().scene_query(), scene);
    assert_eq!(app.document.current_rule_program(), source.as_ref());
    bridge
        .execute(&mut app, Request::Redo { expected: None }, false)
        .unwrap();
    assert!(
        app.document
            .current()
            .occurrences()
            .any(|part| part.name() == "redo-only")
    );
}

#[test]
fn unsupported_supporting_planes_preserve_work_and_do_not_publish_a_distance() {
    let _turn = integration_support::file_turn();
    let (mut app, mut bridge) = fixture(DRILLED);
    let bottom = face_target(&mut app, &mut bridge, "bottom", Some([0.0, 0.0, 1.0]));
    let shelf = face_target(&mut app, &mut bridge, "shelf", Some([0.0, 0.0, -1.0]));
    let side = face_target(&mut app, &mut bridge, "bottom", Some([1.0, 0.0, 0.0]));
    let curved = face_target(&mut app, &mut bridge, "bottom", None);
    // The curved wall is measurable by OCCT, just not a supporting plane.
    assert_distance(
        measure(
            &mut app,
            &mut bridge,
            [curved.clone(), shelf.clone()],
            MeasurementMode::Minimum,
            None,
        ),
        650.0,
        "occt_trimmed_face_distance",
    );
    for (faces, direction) in [
        ([curved, shelf.clone()], Some([0.0, 0.0, 1.0])),
        ([side, shelf.clone()], Some([0.0, 0.0, 1.0])),
        ([bottom.clone(), shelf.clone()], Some([1.0, 0.0, 0.0])),
        ([bottom.clone(), shelf.clone()], Some([0.0, 0.0, 0.0])),
        ([bottom.clone(), shelf.clone()], None),
    ] {
        reject_without_changes(
            &mut app,
            &mut bridge,
            faces,
            MeasurementMode::SupportingPlanes,
            direction,
        );
    }
    assert_distance(
        measure(
            &mut app,
            &mut bridge,
            [bottom, shelf],
            MeasurementMode::SupportingPlanes,
            Some([0.0, 0.0, 1.0]),
        ),
        650.0,
        "exact_supporting_planes",
    );
}

#[test]
fn stale_face_measurement_completion_is_not_published_as_current() {
    let (mut app, mut bridge) = setup();
    let before = app.live_bridge_stamp();
    let (reply, response) = mpsc::sync_channel(1);
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(Ok(json!({"distance_mm":650.0,"state":"verified"})))
        .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    app.new_document();
    bridge.poll_measurement(
        &app,
        &egui::Context::default(),
        MeasurementJob {
            id: 1,
            reply,
            cancelled: cancelled.clone(),
            before,
            started: Instant::now(),
            receiver,
        },
    );
    assert!(!response.try_recv().unwrap().ok);
    assert!(cancelled.load(Ordering::Acquire));
    assert!(app.document.current().occurrences().next().is_none());
}
