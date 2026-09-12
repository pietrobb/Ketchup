use super::*;
use ketchup_application::evaluation::{
    exact_source, exact_worker_candidates, start_exact_evaluation,
};

#[derive(Clone, Copy, Debug)]
enum Failure {
    Incomplete,
    Rejected,
    Disconnected,
}

fn assert_idle_retry(failure: Failure) {
    let (mut app, _bridge) = setup();
    let worker = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    let snapshot = app.document.current();
    let stamp = app.live_bridge_stamp();
    let history = (app.undo_step_count(), app.redo_step_count(), app.is_dirty());
    // Settle egui startup without starting evaluation or receiving OS input.
    app.exact_source = Some(exact_source(&snapshot));
    let (repaint_tx, repaint_rx) = std::sync::mpsc::channel();
    let mut harness = egui_kittest::Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_state(
            move |context, app: &mut KetchupApp| {
                let repaint_tx = repaint_tx.clone();
                context.set_request_repaint_callback(move |info| {
                    let _ = repaint_tx.send(info.delay);
                });
                app.refresh_exact_products(context);
            },
            app,
        );
    harness.run();
    assert_eq!(
        harness.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::MAX
    );

    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    let app = harness.state_mut();
    let task = start_exact_evaluation(
        snapshot,
        &app.container_data,
        &app.exact_results,
        &app.topology_results,
        None,
        move || completed_tx.send(()).unwrap(),
    );
    completed_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    match failure {
        Failure::Incomplete => {}
        Failure::Rejected => task.cancel(),
        Failure::Disconnected => {
            // Consume the only result, then confirm the real sender has closed.
            task.wait(Duration::from_secs(10)).unwrap();
            assert_eq!(
                task.wait(Duration::from_secs(10)).err().as_deref(),
                Some("exact evaluation worker disconnected")
            );
        }
    }
    app.exact_source = None;
    app.exact_task = Some(task);
    app.exact_worker_attempted = true;
    app.exact_worker_path = Some(worker);
    // This is the completion-event frame, not a periodic poll.
    harness.step();
    assert!(harness.state().exact_task.is_none(), "{failure:?}");
    assert!(harness.state().exact_source.is_none(), "{failure:?}");
    let retry_at = harness.state().exact_retry_at.unwrap();
    let delay = harness.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(
        delay > Duration::ZERO && delay <= Duration::from_secs(1),
        "{failure:?}: idle UI needs a finite, non-busy retry deadline, got {delay:?}"
    );
    assert!(
        repaint_rx
            .try_iter()
            .any(|delay| delay <= Duration::from_secs(1))
    );

    // An incidental early frame must re-arm the remaining delay, not reset the
    // one-second backoff or start a worker early.
    harness.step();
    assert_eq!(harness.state().exact_retry_at, Some(retry_at));
    assert!(harness.state().exact_task.is_none());
    let delay = harness.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    assert!(delay > Duration::ZERO && delay <= Duration::from_secs(1));
    for _ in repaint_rx.try_iter() {}
    // Emulate the event loop sleeping until the requested deadline. egui
    // subtracts predicted frame time, hence the small scheduling allowance.
    std::thread::sleep(delay + Duration::from_millis(30));
    harness.step();
    assert!(
        harness.state().exact_task.is_some(),
        "{failure:?}: no retry"
    );

    // From here frames are driven only by repaint callbacks, including the
    // real worker's completion notification. Never poll with repeated steps.
    for _ in 0..8 {
        if harness.state().exact_source.is_some() {
            break;
        }
        let delay = repaint_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(delay, Duration::ZERO);
        harness.step();
    }
    assert!(
        harness.state().exact_source.is_some(),
        "{failure:?}: not recovered"
    );
    assert!(harness.state().exact_task.is_none());
    assert!(harness.state().exact_retry_at.is_none());
    assert!(!harness.state().exact_results.is_empty());
    assert!(!harness.state().topology_results.is_empty());
    assert_eq!(harness.state().live_bridge_stamp(), stamp);
    assert_eq!(
        (
            harness.state().undo_step_count(),
            harness.state().redo_step_count(),
            harness.state().is_dirty()
        ),
        history
    );
    harness.run();
    assert_eq!(
        harness.output().viewport_output[&egui::ViewportId::ROOT].repaint_delay,
        Duration::MAX,
        "successful recovery must stop retry wakeups"
    );
}

#[test]
fn incomplete_evaluation_retries_when_ui_is_idle() {
    assert_idle_retry(Failure::Incomplete);
}

#[test]
fn rejected_evaluation_retries_when_ui_is_idle() {
    assert_idle_retry(Failure::Rejected);
}

#[test]
fn disconnected_evaluation_retries_when_ui_is_idle() {
    assert_idle_retry(Failure::Disconnected);
}
