use std::{io::Result, process::Command};

use super::{CommandWrap, CommandWrapper};

/// Shim wrapper which caps the committed memory of every process in the job.
///
/// Like [`KillOnDrop`](super::KillOnDrop) it only records the request: the `JobObject` wrapper
/// creates its job with `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and this many bytes as
/// `ProcessMemoryLimit`. An allocation past the limit fails inside the child; the parent and the
/// rest of the computer keep their memory.
#[derive(Clone, Copy, Debug)]
pub struct JobMemoryLimit(pub usize);

impl CommandWrapper for JobMemoryLimit {
	fn pre_spawn(&mut self, _command: &mut Command, _core: &CommandWrap) -> Result<()> {
		Ok(())
	}
}
