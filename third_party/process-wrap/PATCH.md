# process-wrap 10.0.0, patched for Ketchup

Unmodified copy of `process-wrap` 10.0.0 from crates.io (Apache-2.0 OR MIT, see the license
files) except two changes: the std API gets the `KillOnDrop` wrapper that the Tokio API already
has, and a `JobMemoryLimit` wrapper (below). The upstream `tests/` directory is left out; this
workspace never builds a dependency's tests.

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

Remove this copy and the `[patch.crates-io]` entry in the root `Cargo.toml` once an upstream
release offers kill-on-close for the std `JobObject`. The change is self-contained and suitable for
an upstream pull request (https://github.com/watchexec/process-wrap).
