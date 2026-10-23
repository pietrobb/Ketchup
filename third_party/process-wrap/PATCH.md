# process-wrap 10.0.0, patched for Ketchup

Unmodified copy of `process-wrap` 10.0.0 from crates.io (Apache-2.0 OR MIT, see the license
files) except seven changes (the fourth: `JobMemoryLimit` on Unix; the fifth: `KillOnDrop` on Linux;
the sixth: a `try_wait` that does not reap the process group; the seventh: `physical_memory_bytes`, below): the std API gets the `KillOnDrop` wrapper that the Tokio API already
has, a `JobMemoryLimit` wrapper, and its job object starts the child without suspending it (below). The upstream `tests/` directory is left out, and
with it the `[[test]]` targets and dev-dependencies of `Cargo.toml`. The unit tests in `src/` run in
CI on Linux and macOS with this directory's own `Cargo.lock` (`.github/workflows/ci.yml`).

- `src/std/kill_on_drop.rs` (new): `KillOnDrop` marker wrapper. `std::process::Command` has no
  kill-on-drop flag, so `pre_spawn` does nothing; the wrapper only records the request.
- `src/std/job_object.rs` (`JobObject::wrap_child`): reads `core.has_wrap::<KillOnDrop>()` and
  passes it to `make_job_object`, exactly as `src/tokio/job_object.rs` does. Upstream passed a
  hard-coded `false`.
- `src/std.rs`: exports `KillOnDrop` behind the existing `kill-on-drop` feature.
- `src/lib.rs`: documentation of the feature and the std usage.

With `command.wrap(KillOnDrop).wrap(JobObject)` the job is created with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Its processes are terminated when the `JobObjectChild` is
dropped, and also when the parent exits, crashes or is killed, because the OS then closes the last
job handle. Without the wrapper the behaviour is unchanged.

Why: `ketchup-scheduler` runs `ketchup-exact-worker` and plugin hosts in a std job object. When a
test binary was killed by its timeout or crashed, the job handle closed but the worker kept running
whenever it was busy with a request (it only exits on stdin EOF between requests). 62 such orphans
accumulated between 9 and 11 October 2026. Ketchup crates `forbid(unsafe_code)`, and setting the job limit needs the
Win32 calls this crate already makes.

## JobMemoryLimit

- `src/std/job_memory_limit.rs` (new): `JobMemoryLimit(bytes)` marker wrapper, like `KillOnDrop`.
- `src/std/job_object.rs` (`JobObject::wrap_child`): reads `core.get_wrap::<JobMemoryLimit>()`.
- `src/windows.rs` (`make_job_object`): takes the limit; when given it sets
  `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and `ProcessMemoryLimit`, and
  `JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION` so a stopped process opens no error dialog. The kill-on-close flag is now or-ed
  in instead of assigned, so both limits can be set. `src/tokio/job_object.rs` passes `None`.
- `src/std.rs`: exports `JobMemoryLimit` next to `JobObject`.

Why: Ketchup evaluates user programs (Starlark) in `ketchup-program-worker`. The interpreter's own
heap limit does not see every allocation (`str.replace`, dict copies, big integers), so a program
could take all the memory of the computer and an allocation failure aborted the editor window. In
a job with a memory limit the allocation fails in the worker only; the editor sees the worker die
and reports a memory limit.

On Unix, where there is no job object, `JobMemoryLimit` is exported too (any feature set) and its
`pre_spawn` sets `RLIMIT_AS` to the limit in the child, through `pre_exec` before `exec`. This
needs nix's `resource` feature (`Cargo.toml`). Ketchup's crates forbid `unsafe`, and `pre_exec`
is unsafe. The limit caps address space, not committed memory as on Windows; the program worker
reserves little beyond what it uses, so the same number serves both (review 2026-10-15 PRG-1).

`src/std/job_memory_limit.rs` also has `physical_memory_bytes()` (`GlobalMemoryStatusEx` on
Windows, which adds `Win32_System_SystemInformation` to the `job-object` feature in `Cargo.toml`;
`sysconf` on Unix), exported next to `JobMemoryLimit`. Ketchup's exact worker, which reads STEP and
IGES files of any size, may commit half of it (review 2026-10-21 LIM-3); both calls are unsafe.

## KillOnDrop on Linux

- `src/std/kill_on_drop.rs`: on Linux `pre_spawn` asks, through `pre_exec` before `exec`, for
  `SIGKILL` when the parent dies (`prctl(PR_SET_PDEATHSIG)`). Elsewhere it still only records the
  request. Needs nix's `process` feature (`Cargo.toml`).

Why: on Unix there is no kill-on-close job, so a plugin kept running after the editor crashed or
was killed (review 2026-10-17 EX-7). The signal follows the spawning thread, which suits a plugin
whose host thread waits for it; Ketchup's pooled exact and program workers watch their parent
themselves instead (`ketchup-scheduler`, `child_process::exit_with_parent`).

## Killing a reaped process group

- `src/std/process_group.rs` (`ProcessGroupChild`): `try_wait` observes the leader's exit with
  `waitid(WNOWAIT)` and leaves it a zombie; only `wait` reaps the leader and the rest of the group.
  After `wait` has reaped the leader, `start_kill` and `signal` do nothing. Once the leader has
  exited, `EPERM` from `killpg` counts as success: macOS refuses to signal a group whose members
  are all zombies, which Linux allows. Two unit tests in the same file. Upstream `try_wait` reaped every process of the group (`waitpid(-pgid)` until `ECHILD`).

Why: Ketchup's plugin host waits for the plugin and then kills its group so no descendant outlives
it. With upstream's reaping `try_wait` that kill failed with `ESRCH`, so every successful plugin run
on Unix ended in a transport error (review 2026-10-17 EX-3); and once the whole group is reaped its
ID is free, so the kill could reach another group that reused it (review 2026-10-19 P3). A zombie
leader keeps the ID reserved until `wait`. `std::process::Child::kill` on a child that has been
reaped returns `Ok` as well.

## No suspended start

- `src/windows.rs` (`job_creation_flags`): the std `JobObject` no longer adds `CREATE_SUSPENDED`
  and never resumes the child itself; the process is assigned to its job right after
  `CreateProcess`. An explicit `CREATE_SUSPENDED` from `CreationFlags` is kept and the process stays
  suspended, as upstream.
- `src/lib.rs`: documentation of the creation flags.

Why: upstream resumes the suspended child with `resume_threads`, which takes a
`CreateToolhelp32Snapshot` of every thread in the system for each spawn. With many processes
starting at once (the UI test suite starts several hundred exact workers) those snapshots
serialised: measured median 218 s, max 278 s from spawn to resume, so every exact evaluation in
the suite missed its deadline (TST-4; the same suite without the job object ran in 16 s). The
cost of not suspending: the child runs for the few microseconds between `CreateProcess` and
`AssignProcessToJobObject` outside its job. Ketchup's workers send nothing and allocate nothing
before their first request, which is written only after the spawn returns, and they start no
processes of their own, so the memory limit and kill-on-close still cover all their work.
std's `ChildExt::main_thread_handle` and `spawn_with_attributes` (job list attribute) would
avoid that window, but both are nightly-only.

Remove this copy and the `[patch.crates-io]` entry in the root `Cargo.toml` once an upstream
release offers kill-on-close for the std `JobObject`. The change is self-contained and suitable for
an upstream pull request (https://github.com/watchexec/process-wrap).
