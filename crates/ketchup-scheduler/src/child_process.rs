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
