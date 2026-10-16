use std::{io::Result, process::Command};

use super::{CommandWrap, CommandWrapper};

/// Shim wrapper which caps the memory of the child.
///
/// On Windows it only records the request, like [`KillOnDrop`](super::KillOnDrop): the
/// `JobObject` wrapper creates its job with `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and this many bytes
/// as `ProcessMemoryLimit`. On Unix the child sets its own `RLIMIT_AS` to this many bytes before
/// it executes the program. Either way an allocation past the limit fails inside the child; the
/// parent and the rest of the computer keep their memory.
#[derive(Clone, Copy, Debug)]
pub struct JobMemoryLimit(pub usize);

impl CommandWrapper for JobMemoryLimit {
	#[cfg(windows)]
	fn pre_spawn(&mut self, _command: &mut Command, _core: &CommandWrap) -> Result<()> {
		Ok(())
	}

	#[cfg(unix)]
	fn pre_spawn(&mut self, command: &mut Command, _core: &CommandWrap) -> Result<()> {
		use nix::sys::resource::{Resource, rlim_t, setrlimit};
		use std::os::unix::process::CommandExt;

		let bytes = rlim_t::try_from(self.0).unwrap_or(rlim_t::MAX);
		// SAFETY: setrlimit(2) is async-signal-safe, and the closure neither allocates nor
		// takes a lock between fork and exec.
		unsafe {
			command.pre_exec(move || {
				setrlimit(Resource::RLIMIT_AS, bytes, bytes).map_err(std::io::Error::from)
			});
		}
		Ok(())
	}
}
