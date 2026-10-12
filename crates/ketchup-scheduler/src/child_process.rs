use process_wrap::std::ChildWrapper;
use std::io;
use std::process::Command;

/// The process id of the process that started a worker.
const PARENT_PID: &str = "KETCHUP_WORKER_PARENT_PID";

/// Names this process as the parent the worker `command` starts must not
/// outlive, also when this process dies before the worker calls
/// [`exit_with_parent`].
pub(crate) fn watched_by_this_process(command: &mut Command) {
    command.env(PARENT_PID, std::process::id().to_string());
}

pub(crate) fn terminate(child: &mut dyn ChildWrapper) -> io::Result<()> {
    // On Unix the kill is a plain signal to the PID. Once the child has been
    // reaped that PID may already belong to an unrelated process, so never
    // signal a child whose exit status we already hold; a child that exits
    // between the check and the signal reports ESRCH, which is also success.
    #[cfg(not(windows))]
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    // Preserve the wrapper: on Windows this terminates the entire Job Object.
    #[cfg(not(windows))]
    match child.start_kill() {
        // ESRCH is 3 on Linux and macOS alike.
        Err(error) if error.raw_os_error() == Some(3) => {}
        result => result?,
    }
    #[cfg(windows)]
    child.start_kill()?;
    // Windows process termination is asynchronous. Waiting for pending kernel I/O
    // can take seconds even after TerminateJobObject succeeds. There are no Unix
    // zombies to reap; dropping the process/job handles releases our resources.
    // Never put that wait (or a completion-port wait) on the cancellation path.
    #[cfg(not(windows))]
    child.wait()?;
    Ok(())
}

/// Ends this worker process once the process that started it is gone, also in
/// the middle of a request: the worker sees its input close only between
/// requests (review 2026-10-17 EX-7). On Windows the parent's kill-on-close
/// job object ends it. A thread rather than `PR_SET_PDEATHSIG`, which follows
/// the spawning thread, and a pooled worker outlives that. The parent is the
/// one [`watched_by_this_process`] named: one that died before this call has
/// already handed the worker to another process (review 2026-10-19 P3).
pub(crate) fn exit_with_parent() {
    #[cfg(unix)]
    {
        use std::os::unix::process::parent_id;
        let parent = std::env::var(PARENT_PID)
            .ok()
            .and_then(|pid| pid.parse().ok())
            .unwrap_or_else(parent_id);
        std::thread::spawn(move || {
            while parent_id() == parent {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            end_now();
        });
    }
}

/// Ends this process at once. `process::exit` would first run the exit
/// handlers and static destructors, OCCT's among them, while the main thread
/// may still be inside OCCT, so the worker could crash or hang instead of
/// ending (review 2026-10-19 P3). SIGKILL runs nothing; with the parent gone,
/// nobody reads the exit status.
#[cfg(unix)]
fn end_now() -> ! {
    use nix::sys::signal::{Signal, raise};
    let _ = raise(Signal::SIGKILL);
    std::process::abort()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::process::ExitStatus;

    #[derive(Debug, Default)]
    struct PendingTeardown {
        terminated: bool,
    }

    impl ChildWrapper for PendingTeardown {
        fn inner(&self) -> &dyn ChildWrapper {
            self
        }
        fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
            self
        }
        fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
            self
        }
        fn start_kill(&mut self) -> io::Result<()> {
            self.terminated = true;
            Ok(())
        }
        fn wait(&mut self) -> io::Result<ExitStatus> {
            panic!("cancellation must not wait for Windows kernel teardown")
        }
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            panic!("cancellation must not consume job completion notifications")
        }
    }

    #[test]
    fn windows_termination_does_not_wait_for_kernel_teardown() {
        let mut child = PendingTeardown::default();
        terminate(&mut child).unwrap();
        assert!(child.terminated);
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    #[test]
    #[ignore = "child process of the test below"]
    fn process_ended_by_the_parent_watch() {
        // Left in stdout's buffer: only an exit that runs the cleanup writes it.
        print!("EXIT_CLEANUP_RAN");
        super::end_now();
    }

    #[test]
    fn the_parent_watch_ends_the_worker_without_its_exit_cleanup() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "child_process::unix_tests::process_ended_by_the_parent_watch",
                "--ignored",
                "--nocapture",
            ])
            .stderr(Stdio::null())
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("EXIT_CLEANUP_RAN"),
            "the exit cleanup ran"
        );
        assert_eq!(output.status.signal(), Some(9), "{:?}", output.status);
    }

    fn test_process(name: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            &format!("child_process::unix_tests::{name}"),
            "--ignored",
            "--nocapture",
        ]);
        command
    }

    #[test]
    #[ignore = "child process of the test below"]
    fn worker_watching_only_after_its_parent_died() {
        // Its stdin, the test's pipe, closes once the test has reaped the parent.
        let _ = std::io::read_to_string(std::io::stdin());
        super::exit_with_parent();
        loop {
            std::thread::park();
        }
    }

    #[test]
    #[ignore = "child process of the test below"]
    fn parent_exiting_right_after_starting_its_worker() {
        let mut command = test_process("worker_watching_only_after_its_parent_died");
        command.stdout(Stdio::null()).stderr(Stdio::null());
        super::watched_by_this_process(&mut command);
        println!("WORKER_PID {}", command.spawn().unwrap().id());
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

    #[cfg(target_os = "linux")]
    #[test]
    fn a_worker_ends_when_its_parent_died_before_it_watched() {
        let mut parent = test_process("parent_exiting_right_after_starting_its_worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let worker_stdin = parent.stdin.take().unwrap();
        // Read to the end: a closed pipe would fail the parent's last line.
        let output = std::io::read_to_string(parent.stdout.take().unwrap()).unwrap();
        let worker_pid: u32 = output
            .lines()
            .find_map(|line| line.strip_prefix("WORKER_PID "))
            .expect("parent reported its worker")
            .parse()
            .unwrap();
        assert!(parent.wait().unwrap().success());
        drop(worker_stdin);
        let _ = Command::new("timeout")
            .args([
                "5",
                "tail",
                &format!("--pid={worker_pid}"),
                "-f",
                "/dev/null",
            ])
            .output();
        let survived = process_is_running(worker_pid);
        if survived {
            let _ = Command::new("kill")
                .args(["-9", &worker_pid.to_string()])
                .output();
        }
        assert!(!survived, "worker {worker_pid} outlived its parent by 5 s");
    }
}
