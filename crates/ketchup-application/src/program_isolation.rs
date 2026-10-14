//! Every evaluation of a user's program in the editor goes through here: it runs
//! in a program worker process (see `ketchup_scheduler::program_worker`), so a
//! program that takes all the memory or never ends cannot take the window down.
use crate::evaluation::{configured_exact_worker, exact_worker_candidates};
use ketchup_program::{CancelToken, Evaluated, ProgramError, Report};
use ketchup_scheduler::program_worker::ProgramWorker;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Idle workers kept for the next evaluation; more run only while evaluations overlap.
const MAX_IDLE_WORKERS: usize = 2;

static IDLE_WORKERS: Mutex<Vec<ProgramWorker>> = Mutex::new(Vec::new());

/// [`ketchup_program::evaluate_cancellable`] in a program worker.
///
/// # Errors
/// As the program, plus `time_limit`, `memory_limit` and `worker_failed`.
pub fn evaluate_program(
    file_name: &str,
    source: &str,
    overrides: &BTreeMap<String, f64>,
    cancel: &CancelToken,
) -> Result<Evaluated, ProgramError> {
    let idle = IDLE_WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop();
    let mut worker = match idle {
        Some(worker) => worker,
        None => new_worker()?,
    };
    let result = worker.evaluate(file_name, source, overrides, cancel);
    let mut idle = IDLE_WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if idle.len() < MAX_IDLE_WORKERS {
        idle.push(worker);
    }
    result
}

/// [`ketchup_program::run`] with the evaluation in a program worker; the report
/// is derived here from the evaluated model.
///
/// # Errors
/// As [`evaluate_program`].
pub fn run_program(
    file_name: &str,
    source: &str,
    overrides: &BTreeMap<String, f64>,
) -> Result<(Evaluated, Report), ProgramError> {
    let evaluated = evaluate_program(file_name, source, overrides, &CancelToken::default())?;
    let report = ketchup_program::report(&evaluated);
    Ok((evaluated, report))
}

const PROGRAM_WORKER: &str = if cfg!(windows) {
    "ketchup-program-worker.exe"
} else {
    "ketchup-program-worker"
};

/// The program worker is installed beside the exact worker.
fn new_worker() -> Result<ProgramWorker, ProgramError> {
    let executable = configured_exact_worker()
        .into_iter()
        .chain(exact_worker_candidates())
        .map(|exact| exact.with_file_name(PROGRAM_WORKER))
        .find(|path| path.is_file())
        .ok_or_else(|| ProgramError {
            code: "worker_failed",
            message:
                "programs cannot be evaluated: ketchup-program-worker is not installed next to Kečup"
                    .to_owned(),
            location: None,
        })?;
    ProgramWorker::new(executable).map_err(|error| ProgramError {
        code: "worker_failed",
        message: format!("programs cannot be evaluated: {error}"),
        location: None,
    })
}
