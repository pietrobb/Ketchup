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

/// Bytes of physical memory of this computer, or `None` when the system does not say; a
/// limit is often a share of it.
pub fn physical_memory_bytes() -> Option<u64> {
	#[cfg(windows)]
	{
		use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
		let mut status = MEMORYSTATUSEX {
			dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).ok()?,
			..Default::default()
		};
		// SAFETY: `status` is a MEMORYSTATUSEX whose dwLength names its size.
		unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
		Some(status.ullTotalPhys)
	}
	#[cfg(unix)]
	{
		use nix::libc::{_SC_PAGESIZE, _SC_PHYS_PAGES, sysconf};
		// SAFETY: sysconf only reads a system value.
		let (pages, page_bytes) = unsafe { (sysconf(_SC_PHYS_PAGES), sysconf(_SC_PAGESIZE)) };
		Some(u64::try_from(pages).ok()? * u64::try_from(page_bytes).ok()?)
	}
}

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
