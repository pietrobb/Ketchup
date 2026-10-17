use std::{io::Result, process::Command};

use super::{CommandWrap, CommandWrapper};

/// Shim wrapper which marks a `Command` as kill-on-drop.
///
/// `std::process::Command` has no kill-on-drop flag of its own, so this wrapper only records the
/// request for other wrappers. Notably the `JobObject` wrapper then creates its job with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: the job's processes are terminated when the
/// `JobObjectChild` is dropped, and also when the spawning process exits or crashes.
///
/// On Linux the child asks for `SIGKILL` when its parent dies (`PR_SET_PDEATHSIG`), before it
/// executes the program. The signal follows the spawning *thread*: spawn from a thread that lives
/// as long as the child, or the child is killed when that thread ends.
#[derive(Clone, Copy, Debug)]
pub struct KillOnDrop;

impl CommandWrapper for KillOnDrop {
	#[cfg(not(target_os = "linux"))]
	fn pre_spawn(&mut self, _command: &mut Command, _core: &CommandWrap) -> Result<()> {
		Ok(())
	}

	#[cfg(target_os = "linux")]
	fn pre_spawn(&mut self, command: &mut Command, _core: &CommandWrap) -> Result<()> {
		use nix::sys::{prctl::set_pdeathsig, signal::Signal};
		use std::os::unix::process::CommandExt;

		// SAFETY: prctl(2) is async-signal-safe, and the closure neither allocates nor takes a
		// lock between fork and exec.
		unsafe {
			command.pre_exec(|| set_pdeathsig(Signal::SIGKILL).map_err(std::io::Error::from));
		}
		Ok(())
	}
}
