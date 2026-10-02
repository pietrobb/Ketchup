use super::*;
use crate::protocol::{ExportReceipt, WorkerReply, WorkerRequest};
use std::io::BufRead as _;

// A real disposable child exercises termination without depending on native geometry.
#[test]
#[ignore = "subprocess used by request timeout tests"]
fn sleeping_worker() {
    println!("WORKER_READY");
    std::thread::sleep(Duration::from_secs(120));
}

fn controlled_worker() -> (
    ExactWorkerClient,
    Receiver<WorkerWriteRequest>,
    Sender<WorkerResponse>,
) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "request_timeout_tests::sleeping_worker",
            "--ignored",
            "--nocapture",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = spawn_exact_worker_command(command).unwrap();
    assert!(
        BufReader::new(child.stdout().take().unwrap())
            .lines()
            .any(|line| line.unwrap() == "WORKER_READY")
    );
    let (write_sender, write_receiver) = mpsc::channel();
    let (response_sender, response_receiver) = mpsc::channel();
    (
        ExactWorkerClient {
            child,
            terminated: false,
            write_sender,
            response_receiver,
            _temp_directory: tempfile::tempdir().unwrap(),
        },
        write_receiver,
        response_sender,
    )
}

#[cfg(windows)]
#[test]
fn request_timeout_terminates_exact_worker_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("escaped-descendant.txt");
    let script = "import subprocess,sys,time\nchild = 'import pathlib,sys,time; time.sleep(0.4); pathlib.Path(sys.argv[1]).write_text(\"escaped\")'\nsubprocess.Popen([sys.executable, '-c', child, sys.argv[1]])\ntime.sleep(30)";
    let mut command = Command::new(ketchup_test_env::python());
    command
        .args([
            std::ffi::OsString::from("-c"),
            std::ffi::OsString::from(script),
        ])
        .arg(&sentinel)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = spawn_exact_worker_command(command).unwrap();
    let (write_sender, writes) = mpsc::channel();
    let (_responses, response_receiver) = mpsc::channel();
    let mut worker = ExactWorkerClient {
        child,
        terminated: false,
        write_sender,
        response_receiver,
        _temp_directory: tempfile::tempdir().unwrap(),
    };
    let responder = std::thread::spawn(move || {
        writes.recv().unwrap().acknowledgment.send(Ok(())).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));

    assert!(matches!(
        worker.request_with_timeout(&WorkerRequest::Hello, &NEVER_CANCELLED, Duration::from_millis(80)),
        Err(WorkerError::RequestTimedOut(timeout)) if timeout == Duration::from_millis(80)
    ));
    responder.join().unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !sentinel.exists(),
        "exact worker descendant survived request timeout and performed a delayed side effect"
    );
}

#[cfg(windows)]
#[test]
fn dropping_exact_worker_terminates_descendants() {
    let directory = tempfile::tempdir().unwrap();
    let sentinel = directory.path().join("escaped-after-drop.txt");
    let script = "import subprocess,sys,time\nchild = 'import pathlib,sys,time; time.sleep(0.4); pathlib.Path(sys.argv[1]).write_text(\"escaped\")'\nsubprocess.Popen([sys.executable, '-c', child, sys.argv[1]])\ntime.sleep(30)";
    let mut command = Command::new(ketchup_test_env::python());
    command
        .args([
            std::ffi::OsString::from("-c"),
            std::ffi::OsString::from(script),
        ])
        .arg(&sentinel)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = spawn_exact_worker_command(command).unwrap();
    let (write_sender, _writes) = mpsc::channel();
    let (_responses, response_receiver) = mpsc::channel();
    let worker = ExactWorkerClient {
        child,
        terminated: false,
        write_sender,
        response_receiver,
        _temp_directory: tempfile::tempdir().unwrap(),
    };
    std::thread::sleep(Duration::from_millis(300));

    drop(worker);
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !sentinel.exists(),
        "exact worker descendant survived client drop and performed a delayed side effect"
    );
}

#[test]
fn oversized_step_xde_worker_output_is_rejected_before_hashing() {
    let (mut worker, writes, responses) = controlled_worker();
    let output = tempfile::NamedTempFile::new().unwrap();
    output.as_file().set_len(MAX_STEP_SOURCE_BYTES + 1).unwrap();
    let source_sha256 = "b".repeat(64);
    let result_fingerprint = "fnv1a64:0123456789abcdef";
    let receipt = WorkerReply::Exported(ExportReceipt {
        result_fingerprint: result_fingerprint.to_owned(),
        sha256: Some("a".repeat(64)),
    });
    let responder = std::thread::spawn(move || {
        let request = writes.recv().unwrap();
        request.acknowledgment.send(Ok(())).unwrap();
        responses
            .send(WorkerResponse::Reply(Box::new(receipt)))
            .unwrap();
    });

    assert!(matches!(
        worker.export_step_xde_part_request_with_cancellation(
            Path::new("source.step"),
            &source_sha256,
            0,
            result_fingerprint,
            output.path(),
            &NEVER_CANCELLED,
        ),
        Err(WorkerError::Transport(message))
            if message == "exact worker STEP output exceeds the bounded 32 MiB envelope"
    ));
    assert!(worker.child.inner_mut().wait().is_ok());
    responder.join().unwrap();
}

#[test]
fn converted_iges_returns_bounded_snapshot_not_later_path_contents() {
    let (mut worker, writes, responses) = controlled_worker();
    let output = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(output.path(), b"verified IGES").unwrap();
    let source_sha256 = "b".repeat(64);
    let receipt = WorkerReply::Exported(ExportReceipt {
        result_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        sha256: None,
    });
    let responder = std::thread::spawn(move || {
        let request = writes.recv().unwrap();
        request.acknowledgment.send(Ok(())).unwrap();
        responses
            .send(WorkerResponse::Reply(Box::new(receipt)))
            .unwrap();
    });

    let verified = worker
        .convert_step_to_iges_request_with_cancellation(
            Path::new("source.step"),
            &source_sha256,
            output.path(),
            &NEVER_CANCELLED,
        )
        .unwrap();
    std::fs::write(output.path(), b"replaced after verification").unwrap();

    assert_eq!(verified, b"verified IGES");
    responder.join().unwrap();
}

#[test]
fn oversized_converted_iges_is_rejected_before_returning_bytes() {
    let (mut worker, writes, responses) = controlled_worker();
    let output = tempfile::NamedTempFile::new().unwrap();
    output.as_file().set_len(MAX_STEP_SOURCE_BYTES + 1).unwrap();
    let source_sha256 = "b".repeat(64);
    let receipt = WorkerReply::Exported(ExportReceipt {
        result_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        sha256: None,
    });
    let responder = std::thread::spawn(move || {
        let request = writes.recv().unwrap();
        request.acknowledgment.send(Ok(())).unwrap();
        responses
            .send(WorkerResponse::Reply(Box::new(receipt)))
            .unwrap();
    });

    assert!(matches!(
        worker.convert_step_to_iges_request_with_cancellation(
            Path::new("source.step"),
            &source_sha256,
            output.path(),
            &NEVER_CANCELLED,
        ),
        Err(WorkerError::Transport(message))
            if message == "exact worker IGES output exceeds the bounded 32 MiB envelope"
    ));
    assert!(worker.child.inner_mut().wait().is_ok());
    responder.join().unwrap();
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
            .send(WorkerResponse::Reply(Box::new(WorkerReply::Done)))
            .unwrap();
    });
    let started = Instant::now();
    assert_eq!(
        worker
            .request_with_timeout(
                &WorkerRequest::Hello,
                &NEVER_CANCELLED,
                EXACT_BREP_GRAPH_REQUEST_TIMEOUT
            )
            .unwrap(),
        WorkerReply::Done
    );
    assert!(started.elapsed() > DEFAULT_WORKER_REQUEST_TIMEOUT);
    responder.join().unwrap();
}

#[test]
fn simple_calls_time_out_at_the_default_budget() {
    let (mut worker, writes, _responses) = controlled_worker();
    let responder = std::thread::spawn(move || {
        writes.recv().unwrap().acknowledgment.send(Ok(())).unwrap();
    });
    let started = Instant::now();
    let error = worker
        .call(
            &WorkerRequest::Hello,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )
        .unwrap_err();
    assert!(
        matches!(error, WorkerError::RequestTimedOut(timeout) if timeout == DEFAULT_WORKER_REQUEST_TIMEOUT)
    );
    assert_eq!(error.to_string(), "worker request timed out after 5000 ms");
    assert!(started.elapsed() >= DEFAULT_WORKER_REQUEST_TIMEOUT);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(worker.child.inner_mut().wait().is_ok());
    responder.join().unwrap();
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
                .request_with_timeout(&WorkerRequest::Hello, &NEVER_CANCELLED, timeout)
                .unwrap_err();
            assert!(matches!(error, WorkerError::RequestTimedOut(actual) if actual == timeout));
            assert_eq!(error.to_string(), "worker request timed out after 80 ms");
            assert!(started.elapsed() >= timeout);
            assert!(worker.child.inner_mut().wait().is_ok());
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
        let _ = responses.send(WorkerResponse::Reply(Box::new(WorkerReply::Done)));
    });
    assert!(matches!(
        worker.request_with_timeout(&WorkerRequest::Hello, &NEVER_CANCELLED, timeout),
        Err(WorkerError::RequestTimedOut(actual)) if actual == timeout
    ));
    assert!(worker.child.inner_mut().wait().is_ok());
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
                worker.request_with_timeout(
                    &WorkerRequest::Hello,
                    &cancelled,
                    EXACT_BREP_GRAPH_REQUEST_TIMEOUT
                ),
                Err(WorkerError::Cancelled)
            ));
            let finished = Instant::now();
            assert!(worker.child.inner_mut().wait().is_ok());
            assert!(
                finished.duration_since(canceller.join().unwrap()) < Duration::from_millis(250)
            );
        });
    }
}

#[cfg(windows)]
#[test]
fn cleanup_after_exit_polling_does_not_wait_for_consumed_job_notifications() {
    let (mut worker, _writes, _responses) = controlled_worker();
    worker.child.start_kill().unwrap();
    worker.child.inner_mut().wait().unwrap();
    for _ in 0..16 {
        assert!(worker.child.try_wait().unwrap().is_some());
    }
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        worker.terminate_worker();
        drop(worker);
        done.send(()).unwrap();
    });
    finished
        .recv_timeout(Duration::from_millis(250))
        .expect("cleanup blocked on an already consumed Windows job notification");
}
#[test]
fn precancelled_graph_request_is_not_written_even_with_zero_budget() {
    let (mut worker, writes, _responses) = controlled_worker();
    assert!(matches!(
        worker.request_with_timeout(
            &WorkerRequest::Hello,
            &AtomicBool::new(true),
            Duration::ZERO
        ),
        Err(WorkerError::Cancelled)
    ));
    assert!(matches!(writes.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(worker.child.inner_mut().wait().is_ok());
}

fn answer(
    writes: Receiver<WorkerWriteRequest>,
    responses: Sender<WorkerResponse>,
    reply: WorkerReply,
) {
    std::thread::spawn(move || {
        let request = writes.recv().unwrap();
        request.acknowledgment.send(Ok(())).unwrap();
        let _ = responses.send(WorkerResponse::Reply(Box::new(reply)));
    });
}

#[test]
fn handshake_refuses_a_worker_from_another_build() {
    let (mut worker, writes, responses) = controlled_worker();
    answer(
        writes,
        responses,
        WorkerReply::Hello {
            protocol: "0".repeat(64),
        },
    );
    assert!(matches!(
        worker.ping(),
        Err(WorkerError::Protocol(message)) if message.contains("differs from scheduler protocol")
    ));
    assert!(worker.terminated);
}

#[test]
fn reply_of_the_wrong_kind_retires_the_worker() {
    let (mut worker, writes, responses) = controlled_worker();
    answer(writes, responses, WorkerReply::Done);
    assert!(matches!(
        worker.ping(),
        Err(WorkerError::Protocol(message)) if message.starts_with("unexpected worker reply Done")
    ));
    assert!(worker.terminated);
}

#[test]
fn geometry_refusal_keeps_the_worker_and_other_failures_retire_it() {
    let (mut worker, writes, responses) = controlled_worker();
    answer(
        writes,
        responses,
        WorkerReply::Failure(protocol::WorkerFailure {
            code: "invalid_profile".to_owned(),
            detail: Some(protocol::FailureDetail {
                diagnostic: "open loop".to_owned(),
                operation: "extrude".to_owned(),
                input_digest: "a".repeat(64),
                backend: "occt".to_owned(),
            }),
        }),
    );
    assert_eq!(
        worker.call(
            &WorkerRequest::PairBegin,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT
        ),
        Err(WorkerError::Geometry(format!(
            "invalid_profile; operation=extrude; diagnostic=open loop; input_digest={}; backend=occt",
            "a".repeat(64)
        )))
    );
    assert!(!worker.terminated);

    let (mut worker, writes, responses) = controlled_worker();
    answer(
        writes,
        responses,
        WorkerReply::Failure(protocol::WorkerFailure::invalid_request()),
    );
    assert!(matches!(
        worker.call(
            &WorkerRequest::PairBegin,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT
        ),
        Err(WorkerError::Protocol(message)) if message.ends_with("invalid_request")
    ));
    assert!(worker.terminated);
}
