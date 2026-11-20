//! The user this process writes as, and becoming another for a while: the calls
//! into the system that take them. Each item calling one allows `unsafe` itself,
//! so the ratchet counts them (review 2026-11-13 P3: one allowance on the module
//! let new blocks in uncounted).

use std::fmt;
use std::io;

/// The user this process writes as.
#[must_use]
#[allow(unsafe_code)]
pub fn effective_user() -> u32 {
    // SAFETY: geteuid takes no pointers and has no preconditions.
    unsafe { libc::geteuid() }
}

/// The group this process writes as.
#[must_use]
#[allow(unsafe_code)]
pub fn effective_group() -> u32 {
    // SAFETY: getegid takes no pointers and has no preconditions.
    unsafe { libc::getegid() }
}

/// Runs `work` as `owner`, user and group, with no other group, and then as root
/// again (review 2026-11-09 R1109-SEC-300: under `sudo` the checks went by path
/// and the write by path again, so a user swapping a folder of theirs for a link
/// to one of root's between them had root write its files there). The system then
/// refuses root's folders and files to it as to that user.
///
/// Only the calling thread becomes the user (review 2026-11-12 R1112-SEC-501:
/// glibc's seteuid changes every thread). A thread `work` starts is that user too
/// and stays so, so `work` starts none: on Linux one found still the user after
/// is an error (review 2026-11-13 r95 P3: that was only written down).
///
/// # Errors
///
/// The thread cannot become `owner`, which is said rather than the failure to
/// take back what it never gave up (review 2026-11-13 P3: macOS answered that);
/// it cannot become root again, which is said with what `work` answered; `work`
/// started a thread that stays the user; or what `work` answers.
pub fn as_user<T>(owner: (u32, u32), work: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    // Declared first, so a panic in the work makes this thread root before it leaves.
    let inside = strays::Inside::enter()?;
    let root = Root::of_this_thread()?;
    if let Err(error) = thread::become_user(owner) {
        // Root again after a change made only in part; macOS makes all or none.
        let back = if thread::ALL_OR_NONE {
            root.kept()
        } else {
            root.restore()
        };
        return Err(match back {
            Ok(()) => error,
            Err(back) => not_root_again(back, Done::NotBecome(owner.0, error)),
        });
    }
    let written = work();
    match (root.restore(), written) {
        (Ok(()), written) => {
            strays::none_left(&inside, owner.0)?;
            written
        }
        (Err(back), Ok(_)) => Err(not_root_again(back, Done::Worked)),
        (Err(back), Err(error)) => Err(not_root_again(back, Done::Failed(error))),
    }
}

/// The threads the work as a user started and left that user (Linux, which shows
/// each thread's user in `/proc`).
#[cfg(target_os = "linux")]
mod strays {
    use std::collections::BTreeSet;
    use std::io;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// The threads in [`super::as_user`] now. A thread joins before it becomes the
    /// user and leaves after it is root again, so under the lock any other thread
    /// that is not root was started by one that was the user.
    static INSIDE: Mutex<BTreeSet<libc::c_long>> = Mutex::new(BTreeSet::new());

    fn inside() -> MutexGuard<'static, BTreeSet<libc::c_long>> {
        INSIDE.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A thread's id from its name under `/proc`.
    fn thread_id(name: &std::ffi::OsStr) -> io::Result<libc::c_long> {
        let text = name.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("thread {name:?} in /proc"),
            )
        })?;
        text.parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// This thread's id, from `/proc/thread-self`, a link to `<pid>/task/<tid>`. Linux
    /// without /proc cannot tell the threads apart, so the work is not done there.
    fn this_thread() -> io::Result<libc::c_long> {
        let link = std::fs::read_link("/proc/thread-self")?;
        thread_id(link.file_name().unwrap_or(link.as_os_str()))
    }

    /// This thread in [`INSIDE`] while it lives, a panic in the work included.
    pub(super) struct Inside(libc::c_long);

    impl Inside {
        pub(super) fn enter() -> io::Result<Self> {
            let thread = this_thread()?;
            inside().insert(thread);
            Ok(Self(thread))
        }
    }

    impl Drop for Inside {
        fn drop(&mut self) {
            inside().remove(&self.0);
        }
    }

    /// No thread outside [`INSIDE`] is `user`, the calling one (`_this`) root again.
    /// A process that is `user` itself has every thread so.
    pub(super) fn none_left(_this: &Inside, user: u32) -> io::Result<()> {
        if user == super::effective_user() {
            return Ok(());
        }
        let inside = inside();
        for thread in std::fs::read_dir("/proc/self/task")? {
            let thread = thread?;
            let id = thread_id(&thread.file_name())?;
            if inside.contains(&id) {
                continue;
            }
            let status = match std::fs::read_to_string(thread.path().join("status")) {
                Ok(status) => status,
                // A thread that ended meanwhile has no status left to read.
                Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            if effective_of(&status) == Some(user) {
                return Err(io::Error::other(format!(
                    "the work as user {user} started thread {id}, which stays that user"
                )));
            }
        }
        Ok(())
    }

    /// The effective user of a `/proc/…/status`: the second of its `Uid:` line.
    fn effective_of(status: &str) -> Option<u32> {
        let uids = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
        uids.split_whitespace().nth(1)?.parse().ok()
    }
}

/// No way to tell a thread's user from another's here.
#[cfg(not(target_os = "linux"))]
mod strays {
    use std::io;

    pub(super) struct Inside;

    impl Inside {
        pub(super) fn enter() -> io::Result<Self> {
            Ok(Self)
        }
    }

    pub(super) fn none_left(_this: &Inside, _user: u32) -> io::Result<()> {
        Ok(())
    }
}

/// That this thread stays the user, after `done`.
fn not_root_again(back: io::Error, done: Done) -> io::Error {
    io::Error::new(back.kind(), NotRootAgain { done, back })
}

/// This thread could not become root again after `done`, for `back`.
#[derive(Debug)]
struct NotRootAgain {
    done: Done,
    back: io::Error,
}

/// What was done before the thread could not become root again.
#[derive(Debug)]
enum Done {
    /// Becoming the user failed, after part of it.
    NotBecome(u32, io::Error),
    /// The work as the user was done.
    Worked,
    /// The work as the user failed so.
    Failed(io::Error),
}

impl fmt::Display for NotRootAgain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.done {
            Done::NotBecome(user, error) => write!(f, "becoming user {user} failed: {error}")?,
            Done::Worked => f.write_str("the work as the user was done")?,
            Done::Failed(error) => write!(f, "{error}")?,
        }
        write!(
            f,
            "; this thread could not become root again: {}",
            self.back
        )
    }
}

impl std::error::Error for NotRootAgain {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.back)
    }
}

/// Becoming a user and root again, for the calling thread alone: the system calls
/// themselves, not glibc's wrappers, which repeat them on every thread. Root's real
/// and saved user stay, so it is root again after.
#[cfg(target_os = "linux")]
mod thread {
    use super::done;
    use std::io;

    /// A failure may leave the groups, or the groups and the group, changed.
    pub(super) const ALL_OR_NONE: bool = false;

    /// Leaves a user, group or groups as they are.
    const UNCHANGED: libc::c_long = -1;

    /// What a raw system call answering 0 when it is done did.
    fn called(answer: libc::c_long) -> io::Result<()> {
        done(if answer == 0 { 0 } else { -1 })
    }

    #[allow(unsafe_code)]
    pub(super) fn become_user(owner: (u32, u32)) -> io::Result<()> {
        let only: [libc::gid_t; 1] = [owner.1];
        let one: libc::c_long = 1;
        // SAFETY: setgroups reads one group from a live local; setresgid and
        // setresuid take no pointers.
        unsafe {
            called(libc::syscall(libc::SYS_setgroups, one, only.as_ptr()))?;
            called(libc::syscall(
                libc::SYS_setresgid,
                UNCHANGED,
                libc::c_long::from(owner.1),
                UNCHANGED,
            ))?;
            called(libc::syscall(
                libc::SYS_setresuid,
                UNCHANGED,
                libc::c_long::from(owner.0),
                UNCHANGED,
            ))
        }
    }

    #[allow(unsafe_code)]
    pub(super) fn become_root(group: libc::gid_t, groups: &[libc::gid_t]) -> io::Result<()> {
        let count = libc::c_long::try_from(groups.len()).map_err(io::Error::other)?;
        let root: libc::c_long = 0;
        // SAFETY: setresuid and setresgid take no pointers; `groups` is live and
        // holds `count` groups.
        unsafe {
            called(libc::syscall(
                libc::SYS_setresuid,
                UNCHANGED,
                root,
                UNCHANGED,
            ))?;
            called(libc::syscall(
                libc::SYS_setresgid,
                UNCHANGED,
                libc::c_long::from(group),
                UNCHANGED,
            ))?;
            called(libc::syscall(libc::SYS_setgroups, count, groups.as_ptr()))
        }
    }
}

/// Becoming a user and root again, for the calling thread alone, as macOS offers
/// it: the thread takes the user's identity, groups included, and gives it back.
#[cfg(target_os = "macos")]
mod thread {
    use super::done;
    use std::io;

    /// A failure changes nothing.
    pub(super) const ALL_OR_NONE: bool = true;

    /// Gives a thread back the process's user and group.
    const KAUTH_UID_NONE: libc::uid_t = libc::uid_t::MAX - 100;
    const KAUTH_GID_NONE: libc::gid_t = libc::gid_t::MAX - 100;

    #[allow(unsafe_code)]
    unsafe extern "C" {
        fn pthread_setugid_np(uid: libc::uid_t, gid: libc::gid_t) -> libc::c_int;
    }

    #[allow(unsafe_code)]
    pub(super) fn become_user(owner: (u32, u32)) -> io::Result<()> {
        // SAFETY: pthread_setugid_np takes no pointers.
        done(unsafe { pthread_setugid_np(owner.0, owner.1) })
    }

    #[allow(unsafe_code)]
    pub(super) fn become_root(_group: libc::gid_t, _groups: &[libc::gid_t]) -> io::Result<()> {
        // SAFETY: pthread_setugid_np takes no pointers.
        done(unsafe { pthread_setugid_np(KAUTH_UID_NONE, KAUTH_GID_NONE) })
    }
}

/// No way to become a user for one thread alone: root writes nothing as another
/// user rather than every thread being them.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod thread {
    use std::io;

    pub(super) const ALL_OR_NONE: bool = true;

    pub(super) fn become_user(_owner: (u32, u32)) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this system cannot write as another user for one thread; run without sudo",
        ))
    }

    pub(super) fn become_root(_group: libc::gid_t, _groups: &[libc::gid_t]) -> io::Result<()> {
        Ok(())
    }
}

/// Root's group and groups, which this thread takes again when dropped.
struct Root {
    group: libc::gid_t,
    groups: Vec<libc::gid_t>,
    restored: bool,
}

impl Root {
    /// The group and groups this thread has now.
    #[allow(unsafe_code)]
    fn of_this_thread() -> io::Result<Self> {
        // SAFETY: getegid takes no pointers; getgroups is given no buffer to ask
        // for the count, then a buffer of that count.
        let (group, groups) = unsafe {
            let mut groups: Vec<libc::gid_t> =
                vec![0; count(libc::getgroups(0, std::ptr::null_mut()))?];
            let filled = libc::getgroups(length(&groups)?, groups.as_mut_ptr());
            groups.truncate(count(filled)?);
            (libc::getegid(), groups)
        };
        Ok(Self {
            group,
            groups,
            restored: false,
        })
    }

    /// This thread is root again.
    fn restore(mut self) -> io::Result<()> {
        self.restored = true;
        thread::become_root(self.group, &self.groups)
    }

    /// This thread never stopped being root.
    fn kept(mut self) -> io::Result<()> {
        self.restored = true;
        Ok(())
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // Unwinding from a panic in the work, with no caller to answer (review
        // 2026-11-10 R1110-SEC-401: the thread was left the user). A thread that
        // cannot be root again is not let run on as the user.
        if !self.restored && thread::become_root(self.group, &self.groups).is_err() {
            std::process::abort();
        }
    }
}

/// What a call answering 0 when it is done did.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn done(answer: libc::c_int) -> io::Result<()> {
    if answer == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The count a call answered; one below zero says it failed.
fn count(answer: libc::c_int) -> io::Result<usize> {
    if answer < 0 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(answer).map_err(io::Error::other)
}

/// How many `groups` there are, as the system takes it: a `size_t` on Linux, an
/// `int` on macOS.
fn length<T: TryFrom<usize>>(groups: &[libc::gid_t]) -> io::Result<T>
where
    T::Error: std::error::Error + Send + Sync + 'static,
{
    T::try_from(groups.len()).map_err(io::Error::other)
}
