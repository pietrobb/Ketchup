use std::{io::Result, process::Command};

use super::{CommandWrap, CommandWrapper};

/// Shim wrapper which marks a `Command` as kill-on-drop.
///
/// `std::process::Command` has no kill-on-drop flag of its own, so this wrapper only records the
/// request for other wrappers. Notably the `JobObject` wrapper then creates its job with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: the job's processes are terminated when the
/// `JobObjectChild` is dropped, and also when the spawning process exits or crashes.
#[derive(Clone, Copy, Debug)]
pub struct KillOnDrop;

impl CommandWrapper for KillOnDrop {
	fn pre_spawn(&mut self, _command: &mut Command, _core: &CommandWrap) -> Result<()> {
		Ok(())
	}
}
