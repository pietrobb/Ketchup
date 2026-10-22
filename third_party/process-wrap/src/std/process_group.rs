use std::{
	io::{Error, Result},
	ops::ControlFlow,
	os::unix::process::{CommandExt, ExitStatusExt},
	process::{Command, ExitStatus},
};

use nix::{
	errno::Errno,
	libc,
	sys::{
		signal::{Signal, killpg},
		wait::WaitPidFlag,
	},
	unistd::Pid,
};
#[cfg(feature = "tracing")]
use tracing::instrument;

use crate::ChildExitStatus;

use super::{ChildWrapper, CommandWrap, CommandWrapper};

/// Wrapper which sets the process group of a `Command`.
///
/// This wrapper is only available on Unix.
///
/// It sets the process group of a [`Command`], either to itself as the leader of a new group, or to
/// an existing one by its PGID. See [setpgid(2)](https://pubs.opengroup.org/onlinepubs/9699919799/functions/setpgid.html).
///
/// Process groups direct signals to all members of the group, and also serve to control job
/// placement in foreground or background, among other actions.
///
/// This wrapper provides a child wrapper: [`ProcessGroupChild`].
#[derive(Clone, Copy, Debug)]
pub struct ProcessGroup {
	leader: Pid,
}

impl ProcessGroup {
	/// Create a process group wrapper setting up a new process group with the command as the leader.
	pub fn leader() -> Self {
		Self {
			leader: Pid::from_raw(0),
		}
	}

	/// Create a process group wrapper attaching the command to an existing process group ID.
	pub fn attach_to(leader: u32) -> Self {
		Self {
			leader: Pid::from_raw(leader as _),
		}
	}
}

/// Wrapper for `Child` which ensures that all processes in the group are reaped.
#[derive(Debug)]
pub struct ProcessGroupChild {
	inner: Box<dyn ChildWrapper>,
	exit_status: ChildExitStatus,
	/// Whether `wait` reaped the leader. Until then the leader, a zombie once it
	/// exits, keeps the group ID reserved, so signalling the group is safe.
	reaped: bool,
	pgid: Pid,
}

impl ProcessGroupChild {
	#[cfg_attr(feature = "tracing", instrument(level = "debug"))]
	pub(crate) fn new(inner: Box<dyn ChildWrapper>, pgid: Pid) -> Self {
		Self {
			inner,
			exit_status: ChildExitStatus::Running,
			reaped: false,
			pgid,
		}
	}

	/// Get the process group ID of this child process.
	///
	/// See: [`man 'setpgid(2)'`](https://www.man7.org/linux/man-pages/man2/setpgid.2.html)
	pub fn pgid(&self) -> u32 {
		self.pgid.as_raw() as _
	}
}

impl CommandWrapper for ProcessGroup {
	#[cfg_attr(feature = "tracing", instrument(level = "debug", skip(self)))]
	fn pre_spawn(&mut self, command: &mut Command, _core: &CommandWrap) -> Result<()> {
		command.process_group(self.leader.as_raw());
		Ok(())
	}

	#[cfg_attr(feature = "tracing", instrument(level = "debug", skip(self)))]
	fn wrap_child(
		&mut self,
		inner: Box<dyn ChildWrapper>,
		_core: &CommandWrap,
	) -> Result<Box<dyn ChildWrapper>> {
		let pgid = Pid::from_raw(i32::try_from(inner.id()).expect("Command PID > i32::MAX"));

		Ok(Box::new(ProcessGroupChild::new(inner, pgid)))
	}
}

impl ProcessGroupChild {
	#[cfg_attr(feature = "tracing", instrument(level = "debug", skip(self)))]
	fn signal_imp(&self, sig: Signal) -> Result<()> {
		// Once the leader is reaped the group ID may name another group.
		if self.reaped {
			return Ok(());
		}
		killpg(self.pgid, sig).map_err(Error::from)
	}

	/// The leader's exit status, observed without reaping it (`WNOWAIT`), so
	/// that a kill after it still reaches only this group (Ketchup review
	/// 2026-10-19 P3).
	fn leader_exit(pid: Pid) -> Result<Option<ExitStatus>> {
		// SAFETY: `siginfo_t` is plain data that `waitid` fills in.
		let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
		let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
		// SAFETY: `info` is a valid, writable `siginfo_t`.
		if unsafe { libc::waitid(libc::P_PID, pid.as_raw() as libc::id_t, &mut info, flags) } == -1 {
			return Err(Error::last_os_error());
		}
		// SAFETY: `waitid` set the child fields, or left them zero (no exit yet).
		let (exited_pid, status) = unsafe { (info.si_pid(), info.si_status()) };
		if exited_pid == 0 {
			return Ok(None);
		}
		// Encode it as `waitpid` does, the form `ExitStatus::from_raw` expects.
		Ok(Some(ExitStatus::from_raw(match info.si_code {
			libc::CLD_EXITED => (status & 0xff) << 8,
			libc::CLD_DUMPED => status | 0x80,
			_ => status,
		})))
	}

	#[cfg_attr(feature = "tracing", instrument(level = "debug"))]
	fn wait_imp(pgid: Pid, flag: WaitPidFlag) -> Result<ControlFlow<Option<ExitStatus>>> {
		// wait for processes in a loop until every process in this group has
		// exited (this ensures that we reap any zombies that may have been
		// created if the parent exited after spawning children, but didn't wait
		// for those children to exit)
		let mut parent_exit_status: Option<ExitStatus> = None;
		loop {
			// we can't use the safe wrapper directly because it doesn't return
			// the raw status, and we need it to convert to the std's ExitStatus
			let mut status: i32 = 0;
			match unsafe {
				libc::waitpid(-pgid.as_raw(), &mut status as *mut libc::c_int, flag.bits())
			} {
				0 => {
					// zero should only happen if WNOHANG was passed in,
					// and means that no processes have yet to exit
					return Ok(ControlFlow::Continue(()));
				}
				-1 => {
					match Errno::last() {
						Errno::ECHILD => {
							// no more children to reap; this is a graceful exit
							return Ok(ControlFlow::Break(parent_exit_status));
						}
						errno => {
							return Err(Error::from(errno));
						}
					}
				}
				pid => {
					// a process exited. was it the parent process that we
					// started? if so, collect the exit signal, otherwise we
					// reaped a zombie process and should continue looping
					if pgid == Pid::from_raw(pid) {
						parent_exit_status = Some(ExitStatus::from_raw(status));
					} else {
						// reaped a zombie child; keep looping
					}
				}
			};
		}
	}
}

impl ChildWrapper for ProcessGroupChild {
	fn inner(&self) -> &dyn ChildWrapper {
		self.inner.as_ref()
	}
	fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
		self.inner.as_mut()
	}
	fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
		self.inner
	}

	#[cfg_attr(feature = "tracing", instrument(level = "debug", skip(self)))]
	fn start_kill(&mut self) -> Result<()> {
		// After `wait` reaped the leader this does nothing, like
		// `std::process::Child::kill` on an exited child.
		self.signal_imp(Signal::SIGKILL)
	}

	#[cfg_attr(feature = "tracing", instrument(level = "debug", skip(self)))]
	fn wait(&mut self) -> Result<ExitStatus> {
		if let (ChildExitStatus::Exited(status), true) = (&self.exit_status, self.reaped) {
			return Ok(*status);
		}

		// always wait for parent to exit first, as by the time it does,
		// it's likely that all its children have already been reaped.
		let status = self.inner.wait()?;
		self.exit_status = ChildExitStatus::Exited(status);
		self.reaped = true;

		// nevertheless, now wait and make sure we reap all children.
		let _ = Self::wait_imp(self.pgid, WaitPidFlag::empty())?;
		Ok(status)
	}

	#[cfg_attr(feature = "tracing", instrument(level = "debug", skip(self)))]
	fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
		if let ChildExitStatus::Exited(status) = &self.exit_status {
			return Ok(Some(*status));
		}

		// `wait` reaps the leader and the rest of the group.
		let exited = Self::leader_exit(self.pgid)?;
		if let Some(status) = exited {
			self.exit_status = ChildExitStatus::Exited(status);
		}
		Ok(exited)
	}

	fn signal(&self, sig: i32) -> Result<()> {
		self.signal_imp(Signal::try_from(sig)?)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::{Duration, Instant};

	/// Ketchup review 2026-10-17 EX-3: the kill that every finished plugin run
	/// ends with, after `try_wait` saw the plugin exit, succeeds, as
	/// `std::process::Child::kill` on an exited child.
	#[test]
	fn killing_a_group_after_try_wait_saw_it_exit_succeeds() {
		let mut command = Command::new("sh");
		command.args(["-c", "exit 0"]);
		let mut command = CommandWrap::from(command);
		command.wrap(ProcessGroup::leader());
		let mut child = command.spawn().unwrap();
		let deadline = Instant::now() + Duration::from_secs(10);
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break status;
			}
			assert!(Instant::now() < deadline, "sh did not exit");
			std::thread::sleep(Duration::from_millis(10));
		};
		assert!(status.success());
		child.start_kill().unwrap();
		assert!(child.wait().unwrap().success());
	}

	/// Ketchup review 2026-10-19 P3: `try_wait` leaves the exited leader a
	/// zombie, which keeps the group ID reserved until `wait`; a kill after
	/// reaping could reach another group that reused the ID.
	#[test]
	fn try_wait_keeps_the_exited_leader_until_wait() {
		let mut command = Command::new("sh");
		command.args(["-c", "exit 3"]);
		let mut command = CommandWrap::from(command);
		command.wrap(ProcessGroup::leader());
		let mut child = command.spawn().unwrap();
		let leader = Pid::from_raw(child.id() as i32);
		let deadline = Instant::now() + Duration::from_secs(10);
		let status = loop {
			if let Some(status) = child.try_wait().unwrap() {
				break status;
			}
			assert!(Instant::now() < deadline, "sh did not exit");
			std::thread::sleep(Duration::from_millis(10));
		};
		assert_eq!(status.code(), Some(3));
		assert_eq!(nix::sys::signal::kill(leader, None), Ok(()), "try_wait reaped the leader");
		child.start_kill().unwrap();
		assert_eq!(child.wait().unwrap().code(), Some(3));
		assert_eq!(nix::sys::signal::kill(leader, None), Err(Errno::ESRCH));
		child.start_kill().unwrap();
	}
}
