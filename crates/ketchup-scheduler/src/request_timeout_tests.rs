use super::*;

// A real disposable child exercises termination without depending on native geometry.
#[test]
#[ignore = "subprocess used by request timeout tests"]
fn sleeping_worker() {
    std::thread::sleep(Duration::from_secs(120));
}

fn controlled_worker() -> (
    ExactWorkerClient,
    Receiver<WorkerWriteRequest>,
    Sender<WorkerResponse>,
) {
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "request_timeout_tests::sleeping_worker",
            "--ignored",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let (write_sender, write_receiver) = mpsc::channel();
    let (response_sender, response_receiver) = mpsc::channel();
    (
        ExactWorkerClient {
            child,
            write_sender,
            response_receiver,
        },
        write_receiver,
        response_sender,
    )
}

#[test]
fn graph_request_budget_accepts_response_beyond_simple_default() {
    assert_eq!(DEFAULT_WORKER_REQUEST_TIMEOUT, Duration::from_secs(5));
    assert_eq!(EXACT_BREP_GRAPH_REQUEST_TIMEOUT, Duration::from_secs(60));
    let (mut worker, writes, responses) = controlled_worker();
    let responder = std::thread::spawn(move || {
        let request = writes.recv().unwrap();
        request.acknowledgment.send(Ok(())).unwrap();
        std::thread::sleep(DEFAULT_WORKER_REQUEST_TIMEOUT + Duration::from_millis(200));
        responses
            .send(WorkerResponse::Line("done".to_owned()))
            .unwrap();
    });
    let started = Instant::now();
    assert_eq!(
        worker
            .request_with_timeout(
                "graph workload",
                &NEVER_CANCELLED,
                EXACT_BREP_GRAPH_REQUEST_TIMEOUT
            )
            .unwrap(),
        "done"
    );
    assert!(started.elapsed() > DEFAULT_WORKER_REQUEST_TIMEOUT);
    responder.join().unwrap();
}

#[test]
fn default_requests_do_not_infer_timeout_from_command_prefix() {
    for command in ["PING", "CAPS EXACT_BREP_GRAPH_V12", "EVAL_BREP_GRAPH_V12"] {
        let (mut worker, writes, _responses) = controlled_worker();
        let responder = std::thread::spawn(move || {
            writes.recv().unwrap().acknowledgment.send(Ok(())).unwrap();
        });
        let started = Instant::now();
        let error = worker.request(command).unwrap_err();
        assert!(
            matches!(error, WorkerError::RequestTimedOut(timeout) if timeout == DEFAULT_WORKER_REQUEST_TIMEOUT)
        );
        assert_eq!(error.to_string(), "worker request timed out after 5000 ms");
        assert!(started.elapsed() >= DEFAULT_WORKER_REQUEST_TIMEOUT);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(worker.child.try_wait().unwrap().is_some());
        responder.join().unwrap();
    }
}

#[test]
fn explicit_timeout_bounds_both_write_and_response_waits() {
    for acknowledge_write in [false, true] {
        let (mut worker, writes, _responses) = controlled_worker();
        let timeout = Duration::from_millis(80);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let request = writes.recv().unwrap();
                if acknowledge_write {
                    request.acknowledgment.send(Ok(())).unwrap();
                }
                // Keep an unacknowledged write connected through timeout.
                std::thread::sleep(Duration::from_millis(200));
            });
            let started = Instant::now();
            let error = worker
                .request_with_timeout("work", &NEVER_CANCELLED, timeout)
                .unwrap_err();
            assert!(matches!(error, WorkerError::RequestTimedOut(actual) if actual == timeout));
            assert_eq!(error.to_string(), "worker request timed out after 80 ms");
            assert!(started.elapsed() >= timeout);
            assert!(worker.child.try_wait().unwrap().is_some());
        });
    }
}

#[test]
fn write_and_response_share_one_timeout_budget() {
    let (mut worker, writes, responses) = controlled_worker();
    let timeout = Duration::from_millis(500);
    let responder = std::thread::spawn(move || {
        let request = writes.recv().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let _ = request.acknowledgment.send(Ok(()));
        std::thread::sleep(Duration::from_millis(300));
        let _ = responses.send(WorkerResponse::Line("too late".to_owned()));
    });
    assert!(matches!(
        worker.request_with_timeout("work", &NEVER_CANCELLED, timeout),
        Err(WorkerError::RequestTimedOut(actual)) if actual == timeout
    ));
    assert!(worker.child.try_wait().unwrap().is_some());
    responder.join().unwrap();
}

#[test]
fn graph_budget_cancellation_interrupts_write_and_response_waits() {
    for acknowledge_write in [false, true] {
        let (mut worker, writes, _responses) = controlled_worker();
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let cancel_flag = &cancelled;
            let canceller = scope.spawn(move || {
                let request = writes.recv().unwrap();
                if acknowledge_write {
                    request.acknowledgment.send(Ok(())).unwrap();
                }
                std::thread::sleep(Duration::from_millis(50));
                let started = Instant::now();
                cancel_flag.store(true, Ordering::Release);
                // Keep the write acknowledgment connected during cancellation.
                std::thread::sleep(Duration::from_millis(50));
                started
            });
            assert!(matches!(
                worker.request_with_timeout("work", &cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT),
                Err(WorkerError::Cancelled)
            ));
            let finished = Instant::now();
            assert!(worker.child.try_wait().unwrap().is_some());
            assert!(
                finished.duration_since(canceller.join().unwrap()) < Duration::from_millis(250)
            );
        });
    }
}

#[test]
fn precancelled_graph_request_is_not_written_even_with_zero_budget() {
    let (mut worker, writes, _responses) = controlled_worker();
    assert!(matches!(
        worker.request_with_timeout("work", &AtomicBool::new(true), Duration::ZERO),
        Err(WorkerError::Cancelled)
    ));
    assert!(matches!(writes.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(worker.child.try_wait().unwrap().is_some());
}
