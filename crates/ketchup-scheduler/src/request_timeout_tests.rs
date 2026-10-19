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
    let mut child = spawn_limited_worker_command(command, None).unwrap();
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
    let child = spawn_limited_worker_command(command, None).unwrap();
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
    let child = spawn_limited_worker_command(command, None).unwrap();
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

const SLEEPING_WORKER_ARGS: [&str; 4] = [
    "--exact",
    "request_timeout_tests::sleeping_worker",
    "--ignored",
    "--nocapture",
];

// Like a real exact or program worker, it ends once its parent is gone.
#[test]
#[ignore = "subprocess used by request timeout tests"]
fn sleeping_worker_watching_its_parent() {
    crate::child_process::exit_with_parent();
    sleeping_worker();
}

// Stand in for a crashed scheduler or plugin host: hold a busy child and get killed from outside.
#[test]
#[ignore = "subprocess used by request timeout tests"]
fn parent_holding_a_busy_worker() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "request_timeout_tests::sleeping_worker_watching_its_parent",
            "--ignored",
            "--nocapture",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hold_busy_child(spawn_limited_worker_command(command, None).unwrap());
}

#[test]
#[ignore = "subprocess used by request timeout tests"]
fn parent_holding_a_busy_plugin() {
    let arguments = SLEEPING_WORKER_ARGS.map(std::ffi::OsString::from);
    let child =
        crate::plugin::spawn_plugin_process(&std::env::current_exe().unwrap(), &arguments, None)
            .unwrap();
    hold_busy_child(child);
}

fn hold_busy_child(mut child: Box<dyn ChildWrapper>) {
    assert!(
        BufReader::new(child.stdout().take().unwrap())
            .lines()
            .any(|line| line.unwrap() == "WORKER_READY")
    );
    println!("WORKER_PID {}", child.id());
    // Held until the test kills this process or closes its stdin.
    let _ = std::io::stdin().read_line(&mut String::new());
}

/// Exact workers and plugins are console programs: started from the windowed
/// app without CREATE_NO_WINDOW, each start opens a new console window. The
/// flag sits beside the kill-on-close job, which must stay.
#[cfg(windows)]
#[test]
fn workers_and_plugins_start_without_a_console_window_inside_their_job() {
    use process_wrap::std::{CreationFlags, JobObject, KillOnDrop};
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;
    let worker = limited_worker_command(Command::new("ketchup-exact-worker"), Some(1 << 30));
    let plugin = crate::plugin::plugin_command(Path::new("python.exe"), &[], None);
    for command in [worker, plugin] {
        assert_eq!(
            command.get_wrap::<CreationFlags>().map(|flags| flags.0),
            Some(CREATE_NO_WINDOW)
        );
        assert!(command.has_wrap::<JobObject>() && command.has_wrap::<KillOnDrop>());
    }
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
}

/// Returns when the process exits (at once if it is already gone) or after 5 s.
#[cfg(windows)]
fn wait_for_exit(pid: u32) {
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Wait-Process -Id {pid} -Timeout 5 -ErrorAction SilentlyContinue"),
        ])
        .output();
}

#[cfg(windows)]
fn kill_process(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output();
}

// A zombie has ended; only its new parent's wait is missing.
#[cfg(target_os = "linux")]
fn process_is_running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit(')')
            .next()
            .is_some_and(|state| !state.trim_start().starts_with('Z'))
    })
}

/// Returns when the process exits (at once if it is already gone) or after 5 s.
#[cfg(target_os = "linux")]
fn wait_for_exit(pid: u32) {
    let _ = Command::new("timeout")
        .args(["5", "tail", &format!("--pid={pid}"), "-f", "/dev/null"])
        .output();
}

#[cfg(target_os = "linux")]
fn kill_process(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_busy_exact_worker_dies_with_its_killed_parent() {
    assert_child_dies_with_its_killed_parent("parent_holding_a_busy_worker");
}

#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_busy_plugin_dies_with_its_killed_host() {
    assert_child_dies_with_its_killed_parent("parent_holding_a_busy_plugin");
}

#[cfg(any(windows, target_os = "linux"))]
fn assert_child_dies_with_its_killed_parent(parent_test: &str) {
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("request_timeout_tests::{parent_test}"),
            "--ignored",
            "--nocapture",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let worker_pid: u32 = BufReader::new(parent.stdout.take().unwrap())
        .lines()
        .find_map(|line| {
            line.unwrap()
                .strip_prefix("WORKER_PID ")
                .map(|pid| pid.parse().unwrap())
        })
        .expect("parent reported its worker");
    assert!(process_is_running(worker_pid));
    assert!(
        parent.try_wait().unwrap().is_none(),
        "parent must still hold its worker"
    );

    parent.kill().unwrap();
    parent.wait().unwrap();
    wait_for_exit(worker_pid);
    let survived = process_is_running(worker_pid);
    if survived {
        kill_process(worker_pid);
    }
    assert!(
        !survived,
        "child {worker_pid} of {parent_test} outlived its killed parent by 5 s"
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

/// A graph request and an exchange request (converting a STEP file) are
/// given their own budgets, not the 5 s of a simple call (EX-2): both workers
/// answer only after that and both answers are taken.
#[test]
fn graph_and_exchange_requests_outlast_the_simple_call_budget() {
    assert_eq!(DEFAULT_WORKER_REQUEST_TIMEOUT, Duration::from_secs(5));
    assert_eq!(EXACT_BREP_GRAPH_REQUEST_TIMEOUT, Duration::from_secs(60));
    let (mut graph_worker, graph_writes, graph_responses) = controlled_worker();
    let (mut exchange_worker, exchange_writes, exchange_responses) = controlled_worker();
    let output = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(output.path(), b"converted IGES").unwrap();
    let receipt = WorkerReply::Exported(ExportReceipt {
        result_fingerprint: "fnv1a64:0123456789abcdef".to_owned(),
        sha256: None,
    });
    let responder = std::thread::spawn(move || {
        for writes in [graph_writes, exchange_writes] {
            writes.recv().unwrap().acknowledgment.send(Ok(())).unwrap();
        }
        std::thread::sleep(DEFAULT_WORKER_REQUEST_TIMEOUT + Duration::from_millis(500));
        graph_responses
            .send(WorkerResponse::Reply(Box::new(WorkerReply::Done)))
            .unwrap();
        exchange_responses
            .send(WorkerResponse::Reply(Box::new(receipt)))
            .unwrap();
    });
    let started = Instant::now();
    std::thread::scope(|scope| {
        let graph = scope.spawn(|| {
            graph_worker.request_with_timeout(
                &WorkerRequest::Hello,
                &NEVER_CANCELLED,
                EXACT_BREP_GRAPH_REQUEST_TIMEOUT,
            )
        });
        let converted = exchange_worker.convert_step_to_iges_request_with_cancellation(
            Path::new("source.step"),
            &"c".repeat(64),
            output.path(),
            &NEVER_CANCELLED,
        );
        assert_eq!(converted, Ok(b"converted IGES".to_vec()));
        assert_eq!(graph.join().unwrap(), Ok(WorkerReply::Done));
    });
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
                cancel_flag.store(true, Ordering::Release);
                // Keep the write acknowledgment connected during cancellation.
                std::thread::sleep(Duration::from_millis(50));
            });
            // Cancelled, not timed out at the graph budget minutes later.
            assert!(matches!(
                worker.request_with_timeout(
                    &WorkerRequest::Hello,
                    &cancelled,
                    EXACT_BREP_GRAPH_REQUEST_TIMEOUT
                ),
                Err(WorkerError::Cancelled)
            ));
            assert!(worker.child.inner_mut().wait().is_ok());
            canceller.join().unwrap();
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

#[test]
fn a_result_too_large_is_refused_with_its_faces_and_keeps_the_worker() {
    let (mut worker, writes, responses) = controlled_worker();
    answer(
        writes,
        responses,
        WorkerReply::TooLarge {
            faces: Some(120_000),
        },
    );
    let error = worker
        .call(
            &WorkerRequest::PairBegin,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )
        .unwrap_err();
    assert_eq!(
        error,
        WorkerError::ResourceLimit {
            faces: Some(120_000)
        }
    );
    assert!(error.to_string().ends_with("(120000 faces)"));
    assert!(!error.permits_restart());
    assert!(!worker.terminated);
}

#[test]
fn only_a_lost_worker_is_asked_again() {
    for lost in [
        WorkerError::Transport("pipe closed".to_owned()),
        WorkerError::WorkerExited,
        WorkerError::MalformedTransport("short frame".to_owned()),
    ] {
        assert!(lost.permits_restart(), "{lost}");
    }
    for deterministic in [
        WorkerError::RequestTimedOut(Duration::from_secs(60)),
        WorkerError::ResponseTooLarge {
            max_bytes: protocol::MAX_REPLY_FRAME_BYTES,
        },
        WorkerError::Protocol("worker refused the request: invalid_request".to_owned()),
        WorkerError::ResourceLimit { faces: None },
        WorkerError::Geometry("invalid_profile".to_owned()),
        WorkerError::Cancelled,
    ] {
        assert!(!deterministic.permits_restart(), "{deterministic}");
    }
}
