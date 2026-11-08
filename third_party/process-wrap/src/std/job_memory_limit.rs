use std::{io::Result, process::Command};

use super::{CommandWrap, CommandWrapper};

/// Shim wrapper which caps the memory of the child.
///
/// On Windows it only records the request, like [`KillOnDrop`](super::KillOnDrop): the
/// `JobObject` wrapper creates its job with `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and this many bytes
/// as `ProcessMemoryLimit`. On Linux the child sets its own `RLIMIT_DATA`, on other Unix but
/// macOS its `RLIMIT_AS`, to this many bytes before it executes the program. Either way an
/// allocation past the limit fails inside the child; the parent and the rest of the computer keep
/// their memory.
///
/// macOS refuses an `RLIMIT_AS` below the address space a process already reserves (hundreds of
/// GiB with the shared cache), and that differs before and after `exec`. There a thread of the
/// parent samples the child's physical footprint and kills its process group past the limit
/// ([`memory_limit_exceeded`] then says so), and a child that calls
/// [`apply_inherited_memory_limit`] caps its own address space at the limit above what it
/// reserves then, so a huge allocation fails at once, before any of it is written.
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

	#[cfg(target_os = "macos")]
	fn pre_spawn(&mut self, command: &mut Command, _core: &CommandWrap) -> Result<()> {
		command.env(macos::INHERITED_LIMIT, self.0.to_string());
		Ok(())
	}

	#[cfg(target_os = "macos")]
	fn wrap_child(
		&mut self,
		inner: Box<dyn super::ChildWrapper>,
		_core: &CommandWrap,
	) -> Result<Box<dyn super::ChildWrapper>> {
		Ok(Box::new(macos::FootprintWatchedChild::start(inner, self.0)?))
	}

	#[cfg(all(unix, not(target_os = "macos")))]
	fn pre_spawn(&mut self, command: &mut Command, _core: &CommandWrap) -> Result<()> {
		use nix::sys::resource::{Resource, rlim_t, setrlimit};
		use std::os::unix::process::CommandExt;

		let bytes = rlim_t::try_from(self.0).unwrap_or(rlim_t::MAX);
		// Linux (4.7 and later) counts in RLIMIT_DATA the private writable memory a process
		// maps, not the address space it only reserves, as glibc does 64 MiB per thread for
		// its malloc arenas; elsewhere RLIMIT_AS is the cap there is.
		#[cfg(target_os = "linux")]
		let resource = Resource::RLIMIT_DATA;
		#[cfg(not(target_os = "linux"))]
		let resource = Resource::RLIMIT_AS;
		// SAFETY: setrlimit(2) is async-signal-safe, and the closure neither allocates nor
		// takes a lock between fork and exec.
		unsafe {
			command.pre_exec(move || {
				setrlimit(resource, bytes, bytes).map_err(std::io::Error::from)
			});
		}
		Ok(())
	}
}

/// Whether the footprint watch killed `child` past its [`JobMemoryLimit`]. Only on macOS; elsewhere
/// the child's own allocation fails instead and this is `false`.
pub fn memory_limit_exceeded(child: &dyn super::ChildWrapper) -> bool {
	#[cfg(target_os = "macos")]
	{
		macos::exceeded(child)
	}
	#[cfg(not(target_os = "macos"))]
	{
		let _ = child;
		false
	}
}

/// Caps the address space of this process, a child started with a [`JobMemoryLimit`], at that
/// limit above what it reserves now. Only on macOS, where the parent cannot set it before `exec`;
/// elsewhere, and in a process started without the limit, this does nothing.
///
/// # Errors
/// The limit the parent passed is not a number, or the system refused it.
pub fn apply_inherited_memory_limit() -> Result<()> {
	#[cfg(target_os = "macos")]
	macos::apply_inherited_limit()?;
	Ok(())
}

/// What the footprint watch does after one sample of its child.
#[cfg(unix)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
enum Watch {
	/// The child is within its limit: sample again.
	Within,
	/// The child is past its limit: kill it.
	Exceeded,
	/// The child was reaped, or its pid names another process now: nothing is left to watch.
	Gone,
	/// The footprint could not be read: the child would run without its limit, so it is killed
	/// all the same (Ketchup review 2026-11-04: such a sample ended the watch).
	Unwatchable,
}

/// Judges one sample, the child's start time and footprint, of a child started at `started`.
#[cfg(unix)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn judge(sample: &Result<(u64, u64)>, started: u64, limit: u64) -> Watch {
	match sample {
		Ok((now_started, _)) if *now_started != started => Watch::Gone,
		Ok((_, bytes)) if *bytes > limit => Watch::Exceeded,
		Ok(_) => Watch::Within,
		Err(error) if error.raw_os_error() == Some(nix::libc::ESRCH) => Watch::Gone,
		Err(_) => Watch::Unwatchable,
	}
}

#[cfg(all(test, unix))]
mod watch_tests {
	use super::{Watch, judge};
	use std::io::Error;

	#[test]
	fn a_sample_that_cannot_be_read_stops_the_child_and_only_a_gone_one_ends_the_watch() {
		assert_eq!(judge(&Ok((7, 100)), 7, 200), Watch::Within);
		assert_eq!(judge(&Ok((7, 300)), 7, 200), Watch::Exceeded);
		assert_eq!(judge(&Ok((8, 300)), 7, 200), Watch::Gone);
		let gone = Error::from_raw_os_error(nix::libc::ESRCH);
		assert_eq!(judge(&Err(gone), 7, 200), Watch::Gone);
		let refused = Error::from_raw_os_error(nix::libc::EPERM);
		assert_eq!(judge(&Err(refused), 7, 200), Watch::Unwatchable);
	}
}

#[cfg(target_os = "macos")]
mod macos {
	use std::{
		any::Any,
		io::{Error, ErrorKind, Result},
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
		},
		time::Duration,
	};

	use nix::{
		libc,
		sys::{
			resource::{Resource, getrlimit, setrlimit},
			signal::{Signal, kill, killpg},
		},
		unistd::{Pid, getpgid},
	};

	use super::super::ChildWrapper;

	/// The variable through which the parent hands the limit to its child.
	pub(super) const INHERITED_LIMIT: &str = "PROCESS_WRAP_MEMORY_LIMIT";

	/// How often the footprint is sampled; memory written between two samples is the overshoot.
	const SAMPLE_EVERY: Duration = Duration::from_millis(50);

	/// A child whose physical footprint a thread watches.
	#[derive(Debug)]
	pub(super) struct FootprintWatchedChild {
		inner: Box<dyn ChildWrapper>,
		exceeded: Arc<AtomicBool>,
		_watch: StopOnDrop,
	}

	#[derive(Debug)]
	struct StopOnDrop(Arc<AtomicBool>);

	impl Drop for StopOnDrop {
		fn drop(&mut self) {
			self.0.store(true, Ordering::Relaxed);
		}
	}

	impl FootprintWatchedChild {
		pub(super) fn start(inner: Box<dyn ChildWrapper>, limit: usize) -> Result<Self> {
			Self::start_sampling(inner, limit, footprint)
		}

		/// A child that `footprint` cannot be read of from the start would run without its
		/// limit, so it is killed with its group and reaped, and the start fails (Ketchup
		/// review 2026-11-03 PL-141: it was left running).
		pub(super) fn start_sampling(
			mut inner: Box<dyn ChildWrapper>,
			limit: usize,
			footprint: fn(libc::pid_t) -> Result<(u64, u64)>,
		) -> Result<Self> {
			let pid = match libc::pid_t::try_from(inner.id()) {
				Ok(pid) => pid,
				Err(error) => {
					let _ = inner.start_kill();
					let _ = inner.wait();
					return Err(Error::new(ErrorKind::InvalidInput, error));
				}
			};
			let started = match footprint(pid) {
				Ok((started, _)) => started,
				Err(error) => {
					kill_group(pid);
					let _ = inner.wait();
					return Err(error);
				}
			};
			let limit = u64::try_from(limit).unwrap_or(u64::MAX);
			let exceeded = Arc::new(AtomicBool::new(false));
			let stop = Arc::new(AtomicBool::new(false));
			let (watch_exceeded, watch_stop) = (Arc::clone(&exceeded), Arc::clone(&stop));
			std::thread::Builder::new()
				.name(format!("memory limit of {pid}"))
				.spawn(move || {
					while !watch_stop.load(Ordering::Relaxed) {
						// The start time tells the child from a process that reused its pid
						// once it was reaped.
						let sample = footprint(pid);
						match super::judge(&sample, started, limit) {
							super::Watch::Within => {}
							super::Watch::Exceeded => {
								watch_exceeded.store(true, Ordering::Relaxed);
								kill_group(pid);
								return;
							}
							super::Watch::Gone => return,
							super::Watch::Unwatchable => {
								if let Err(error) = sample {
									eprintln!(
										"process-wrap: the memory of process {pid} cannot be read ({error}), so it is stopped"
									);
								}
								kill_group(pid);
								return;
							}
						}
						std::thread::sleep(SAMPLE_EVERY);
					}
				})?;
			Ok(Self {
				inner,
				exceeded,
				_watch: StopOnDrop(stop),
			})
		}
	}

	impl ChildWrapper for FootprintWatchedChild {
		fn inner(&self) -> &dyn ChildWrapper {
			&*self.inner
		}

		fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
			&mut *self.inner
		}

		fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
			self.inner
		}
	}

	/// The child's start time and physical footprint (Activity Monitor's "Memory": written,
	/// compressed and swapped memory, not address space only reserved).
	fn footprint(pid: libc::pid_t) -> Result<(u64, u64)> {
		// SAFETY: `rusage_info_v4` is plain data that `proc_pid_rusage` fills in.
		let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
		// SAFETY: `info` is a writable `rusage_info_v4`, the struct of the V4 flavour.
		let result = unsafe {
			libc::proc_pid_rusage(
				pid,
				libc::RUSAGE_INFO_V4,
				std::ptr::from_mut(&mut info).cast::<libc::rusage_info_t>(),
			)
		};
		if result != 0 {
			return Err(Error::last_os_error());
		}
		Ok((info.ri_proc_start_abstime, info.ri_phys_footprint))
	}

	/// Kills the child's process group when it leads one, else the child alone.
	fn kill_group(pid: libc::pid_t) {
		let pid = Pid::from_raw(pid);
		let _ = if getpgid(Some(pid)) == Ok(pid) {
			killpg(pid, Signal::SIGKILL)
		} else {
			kill(pid, Signal::SIGKILL)
		};
	}

	pub(super) fn exceeded(child: &dyn ChildWrapper) -> bool {
		let mut layer = child;
		loop {
			if let Some(watched) = (layer as &dyn Any).downcast_ref::<FootprintWatchedChild>() {
				return watched.exceeded.load(Ordering::Relaxed);
			}
			let next = layer.inner();
			if std::ptr::addr_eq(layer, next) {
				return false;
			}
			layer = next;
		}
	}

	pub(super) fn apply_inherited_limit() -> Result<()> {
		let Ok(limit) = std::env::var(INHERITED_LIMIT) else {
			return Ok(());
		};
		let limit: u64 = limit
			.parse()
			.map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
		let (_, hard) = getrlimit(Resource::RLIMIT_AS)?;
		let bytes = reserved_bytes()?.saturating_add(limit).min(hard);
		setrlimit(Resource::RLIMIT_AS, bytes, bytes)?;
		Ok(())
	}

	/// Address space this process reserves, the shared cache included.
	fn reserved_bytes() -> Result<u64> {
		// SAFETY: `proc_taskinfo` is plain data that `proc_pidinfo` fills in.
		let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
		let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_taskinfo>())
			.map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
		// SAFETY: `info` is a writable `proc_taskinfo` of `size` bytes.
		let written = unsafe {
			libc::proc_pidinfo(
				std::process::id().cast_signed(),
				libc::PROC_PIDTASKINFO,
				0,
				std::ptr::from_mut(&mut info).cast(),
				size,
			)
		};
		if written != size {
			return Err(Error::last_os_error());
		}
		Ok(info.pti_virtual_size)
	}
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
	use super::*;
	use std::{
		os::unix::process::ExitStatusExt,
		process::Stdio,
		time::{Duration, Instant},
	};

	fn test_process(name: &str, limit: usize) -> CommandWrap {
		let mut command = Command::new(std::env::current_exe().unwrap());
		command
			.args(["--exact", &format!("std::job_memory_limit::tests::{name}")])
			.args(["--ignored", "--nocapture"])
			.stdout(Stdio::piped())
			.stderr(Stdio::null());
		let mut command = CommandWrap::from(command);
		command.wrap(super::super::ProcessGroup::leader());
		command.wrap(JobMemoryLimit(limit));
		command
	}

	#[test]
	#[ignore = "child process of the test below"]
	fn process_writing_more_memory_than_its_limit() {
		// Written, so it counts in the physical footprint, not only in the address space; and
		// with noise, which macOS's memory compressor cannot shrink below the limit (Ketchup
		// review 2026-11-04: a block of ones might stay under it, compressed).
		let mut state = 0x9E37_79B9_7F4A_7C15_u64;
		let block: Vec<u64> = (0..(512_usize << 20) / 8)
			.map(|_| {
				state ^= state << 13;
				state ^= state >> 7;
				state ^= state << 17;
				state
			})
			.collect();
		std::hint::black_box(&block);
		std::thread::sleep(Duration::from_secs(120));
	}

	/// Ketchup review 2026-11-01 PL-61: a child that never asks for its limit is still
	/// killed, with its group, once it uses more memory than the limit.
	#[test]
	fn a_child_writing_more_memory_than_its_limit_is_killed() {
		let mut child = test_process("process_writing_more_memory_than_its_limit", 256 << 20)
			.spawn()
			.unwrap();
		// Only stops waiting on a child nothing ended.
		let deadline = Instant::now() + Duration::from_secs(60);
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break Some(status);
			}
			if Instant::now() > deadline {
				child.start_kill().unwrap();
				break None;
			}
			std::thread::sleep(Duration::from_millis(50));
		};
		assert_eq!(status.and_then(|status| status.signal()), Some(9));
		assert!(memory_limit_exceeded(&*child));
	}

	/// Ketchup review 2026-11-03 PL-141: a child whose footprint could not be read at once
	/// was left running without its limit.
	#[test]
	fn a_child_whose_footprint_cannot_be_read_at_start_is_killed() {
		let mut command = Command::new("/bin/sleep");
		command.arg("60");
		let mut command = CommandWrap::from(command);
		command.wrap(super::super::ProcessGroup::leader());
		let child = command.spawn().unwrap();
		let pid = nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap());
		let refused = |_pid| Err(std::io::Error::from_raw_os_error(nix::libc::EPERM));
		let started = macos::FootprintWatchedChild::start_sampling(child, 256 << 20, refused);
		assert!(started.is_err());
		assert_eq!(
			nix::sys::signal::kill(pid, None),
			Err(nix::errno::Errno::ESRCH)
		);
	}

	#[test]
	#[ignore = "child process of the test below"]
	fn process_reserving_more_memory_than_its_limit() {
		apply_inherited_memory_limit().unwrap();
		let mut small = Vec::<u8>::new();
		let mut huge = Vec::<u8>::new();
		println!(
			"small {} huge {}",
			small.try_reserve_exact(64 << 20).is_ok(),
			huge.try_reserve_exact(300 << 30).is_ok()
		);
	}

	/// Ketchup review 2026-11-01 PL-61: memory only reserved is never in the footprint, so
	/// the child caps its own address space and a huge reservation fails at once.
	#[test]
	fn a_child_keeping_its_limit_cannot_reserve_more_memory() {
		let mut child = test_process("process_reserving_more_memory_than_its_limit", 256 << 20)
			.spawn()
			.unwrap();
		let output = std::io::read_to_string(child.stdout().take().unwrap()).unwrap();
		assert!(child.wait().unwrap().success());
		assert!(output.contains("small true huge false"), "{output}");
		assert!(!memory_limit_exceeded(&*child));
	}
}
