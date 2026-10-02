use std::sync::mpsc;
use std::time::{Duration, Instant};

use ketchup_scheduler::background::{BackgroundTask, TaskContext, TaskEvent, TaskStop};

const PATIENCE: Duration = Duration::from_secs(5);

/// A job that blocks until it is cancelled, then reports that it saw it.
fn until_cancelled(seen: mpsc::Sender<()>) -> impl FnOnce(&TaskContext<u8, ()>) + Send {
    move |context| {
        while !context.is_cancelled() {
            std::thread::yield_now();
        }
        let _ = seen.send(());
    }
}

#[test]
fn progress_arrives_in_order_before_the_result() {
    let task = BackgroundTask::spawn(
        "progress",
        PATIENCE,
        |context: &TaskContext<u8, &str>| {
            for step in 0..3 {
                context.progress(step).unwrap();
            }
            "done"
        },
        || {},
    )
    .unwrap();
    let mut events = Vec::new();
    loop {
        match task.next_event(PATIENCE).unwrap() {
            TaskEvent::Progress(step) => events.push(step),
            TaskEvent::Finished(result) => {
                assert_eq!(result, Ok("done"));
                break;
            }
        }
    }
    assert_eq!(events, [0, 1, 2]);
}

#[test]
fn wait_skips_progress_and_returns_the_result() {
    let task = BackgroundTask::spawn(
        "wait",
        PATIENCE,
        |context: &TaskContext<u8, u32>| {
            context.progress(1).unwrap();
            42
        },
        || {},
    )
    .unwrap();
    assert_eq!(task.wait(PATIENCE), Ok(42));
}

#[test]
fn cancel_sets_the_flag_the_job_sees_and_ends_cancelled() {
    let (seen, saw_cancel) = mpsc::channel();
    let task = BackgroundTask::spawn("cancel", PATIENCE, until_cancelled(seen), || {}).unwrap();
    task.cancel();
    assert_eq!(task.wait(PATIENCE), Err(TaskStop::Cancelled));
    saw_cancel.recv_timeout(PATIENCE).unwrap();
}

#[test]
fn passing_the_deadline_cancels_the_job_and_ends_timed_out() {
    let (seen, saw_cancel) = mpsc::channel();
    let task =
        BackgroundTask::spawn("deadline", Duration::ZERO, until_cancelled(seen), || {}).unwrap();
    saw_cancel.recv_timeout(PATIENCE).unwrap();
    assert_eq!(task.wait(PATIENCE), Err(TaskStop::TimedOut));
}

#[test]
fn dropping_the_task_cancels_the_job() {
    let (seen, saw_cancel) = mpsc::channel();
    let task = BackgroundTask::spawn("drop", PATIENCE, until_cancelled(seen), || {}).unwrap();
    drop(task);
    saw_cancel.recv_timeout(PATIENCE).unwrap();
}

#[test]
fn a_result_after_cancel_reads_as_cancelled() {
    let (release, released) = mpsc::channel::<()>();
    let (finished, job_finished) = mpsc::channel();
    let task = BackgroundTask::spawn(
        "late",
        PATIENCE,
        move |_: &TaskContext<u8, u32>| {
            released.recv().unwrap();
            7
        },
        move || finished.send(()).unwrap(),
    )
    .unwrap();
    release.send(()).unwrap();
    job_finished.recv_timeout(PATIENCE).unwrap();
    task.cancel();
    assert!(matches!(
        task.poll(),
        Ok(TaskEvent::Finished(Err(TaskStop::Cancelled)))
    ));
}

#[test]
fn a_timed_out_wait_and_drop_do_not_block_on_an_unresponsive_job() {
    let (release, released) = mpsc::channel::<()>();
    let task = BackgroundTask::spawn(
        "unresponsive",
        PATIENCE,
        move |_: &TaskContext<u8, ()>| {
            let _ = released.recv();
        },
        || {},
    )
    .unwrap();
    let started_at = Instant::now();
    let result = task.wait(Duration::ZERO);
    let elapsed = started_at.elapsed();
    drop(release);
    assert_eq!(result, Err(TaskStop::TimedOut));
    assert!(elapsed < Duration::from_millis(500), "elapsed={elapsed:?}");
}

#[test]
fn a_slow_finished_callback_does_not_delay_the_result() {
    let (release, released) = mpsc::channel::<()>();
    let task = BackgroundTask::spawn(
        "callback",
        PATIENCE,
        |_: &TaskContext<u8, u32>| 1,
        move || {
            let _ = released.recv();
        },
    )
    .unwrap();
    assert_eq!(task.wait(PATIENCE), Ok(1));
    drop(release);
}
