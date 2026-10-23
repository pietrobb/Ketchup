use process_wrap::std::ChildWrapper;
use std::io;

pub(crate) fn terminate(child: &mut dyn ChildWrapper) -> io::Result<()> {
    // Preserve the wrapper: on Windows this terminates the entire Job Object.
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
/// the spawning thread, and a pooled worker outlives that.
pub(crate) fn exit_with_parent() {
    #[cfg(unix)]
    {
        use std::os::unix::process::parent_id;
        let parent = parent_id();
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
}
