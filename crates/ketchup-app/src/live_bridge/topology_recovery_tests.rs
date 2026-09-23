use super::*;
use ketchup_application::evaluation::{
    EvidenceStatus, exact_worker_candidates, publish_exact_products, start_exact_evaluation,
};
use ketchup_core::exact_product::{ExactBodyPackage, ExactFeatureChainRequest};
use ketchup_scheduler::ExactWorkerSupervisor;

#[test]
fn topology_retry_preserves_render_and_recovers_live_queries_without_edit() {
    let (mut app, mut bridge) = setup();
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    let snapshot = app.document.current();
    let request = ExactFeatureChainRequest::from_snapshot_for_producer(
        &snapshot,
        DefinitionId(1),
        FeatureId(2),
    )
    .unwrap();
    let render = Arc::new(ExactBodyPackage::from(
        ExactWorkerSupervisor::spawn(&worker)
            .unwrap()
            .evaluate_rectangle(&request)
            .unwrap(),
    ));
    // Inject the state left by a successful rectangle request followed by a
    // failed topology request. The render is real worker evidence, not a mock.
    app.exact_results
        .insert_current(&snapshot, Arc::clone(&render))
        .unwrap();
    assert!(app.topology_results.is_empty());
    let stamp = app.live_bridge_stamp();
    let history = (app.undo_step_count(), app.redo_step_count(), app.is_dirty());
    for kind in [EntityKind::Faces, EntityKind::Edges] {
        let page = bridge
            .execute(
                &mut app,
                Request::Query {
                    expected: Some(stamp.clone()),
                    query: serde_json::from_value(json!({"kind": kind, "limit": 100})).unwrap(),
                },
                false,
            )
            .unwrap();
        assert!(page["items"].as_array().unwrap().is_empty(), "{page}");
    }

    // A retry with an unavailable worker must retain the render and diagnose
    // topology, rather than silently report this producer as complete.
    let task = start_exact_evaluation(
        snapshot.clone(),
        &app.container_data,
        &app.exact_results,
        &app.topology_results,
        None,
        || {},
    );
    let products = task.wait(Duration::from_secs(10)).unwrap();
    let report = publish_exact_products(
        &mut app.document,
        &mut app.exact_results,
        &mut app.topology_results,
        &task,
        products,
    )
    .unwrap();
    assert!(
        report.complete && !report.topology_complete && report.needs_retry(),
        "{report:?}"
    );
    assert_eq!(report.producers[0].render, EvidenceStatus::Current);
    assert!(
        matches!(&report.producers[0].topology, EvidenceStatus::Failed { reason }
        if reason == "exact worker unavailable")
    );
    assert_eq!(
        app.exact_results.values().next().unwrap().as_ref(),
        render.as_ref()
    );

    // Exercise desktop completion through egui_kittest, without OS input.
    app.exact_task = Some(start_exact_evaluation(
        snapshot,
        &app.container_data,
        &app.exact_results,
        &app.topology_results,
        None,
        || {},
    ));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app
        .exact_task
        .as_ref()
        .unwrap()
        .finished
        .load(Ordering::Acquire)
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut harness = egui_kittest::Harness::new_state(
        |context, app: &mut KetchupApp| app.refresh_exact_products(context),
        app,
    );
    assert!(harness.state().exact_source.is_none());
    assert!(harness.state().exact_retry_at.is_some());
    assert!(harness.state().topology_results.is_empty());
    harness.state_mut().exact_worker_path = Some(worker);
    harness.state_mut().exact_worker_attempted = true;
    // Deadline scheduling in an idle UI is separately covered by goal #2005.
    harness.state_mut().exact_retry_at = Some(Instant::now());
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.state().exact_source.is_none() {
        assert!(Instant::now() < deadline, "topology recovery deadline");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    let app = harness.state_mut();
    assert!(app.exact_retry_at.is_none());
    assert_eq!(
        app.exact_results.values().next().unwrap().as_ref(),
        render.as_ref()
    );
    assert!(app.topology_results.is_bound_to(&app.document.current()));
    for kind in [EntityKind::Faces, EntityKind::Edges] {
        let result = bridge
            .execute(
                app,
                Request::Query {
                    expected: Some(stamp.clone()),
                    query: serde_json::from_value(json!({"kind": kind, "limit": 100})).unwrap(),
                },
                false,
            )
            .unwrap();
        assert!(!result["items"].as_array().unwrap().is_empty(), "{result}");
        assert_eq!(result["coverage"]["geometry_evaluated"], true);
    }
    assert_eq!(app.live_bridge_stamp(), stamp);
    assert_eq!(
        (app.undo_step_count(), app.redo_step_count(), app.is_dirty()),
        history
    );
    let topology = app.topology_results.values().cloned().collect::<Vec<_>>();
    let task = start_exact_evaluation(
        app.document.current(),
        &app.container_data,
        &app.exact_results,
        &app.topology_results,
        None,
        || {},
    );
    let products = task.wait(Duration::from_secs(10)).unwrap();
    let cached = publish_exact_products(
        &mut app.document,
        &mut app.exact_results,
        &mut app.topology_results,
        &task,
        products,
    )
    .unwrap();
    assert!(cached.complete && cached.topology_complete && !cached.needs_retry());
    assert_eq!(cached.producers[0].topology, EvidenceStatus::Current);
    assert_eq!(
        app.topology_results.values().cloned().collect::<Vec<_>>(),
        topology
    );
}
