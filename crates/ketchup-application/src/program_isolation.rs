//! Every evaluation of a user's program in the editor, every check of its model
//! and every DWG file it reads go through here: they run in a program worker
//! process (see `ketchup_scheduler::program_worker`), so a program that takes
//! all the memory or never ends, a model too large to check, or a damaged DWG
//! file cannot take the window down.
use crate::evaluation::{configured_exact_worker, exact_worker_candidates};
use ketchup_dwg::DwgDrawing;
use ketchup_program::{
    CancelToken, Evaluated, ExactShapes, ProgramError, ProgramModel, Refined, Report,
};
use ketchup_scheduler::WorkerError;
pub use ketchup_scheduler::program_worker::{CheckDeadline, PROGRAM_CHECK_TIME};
use ketchup_scheduler::program_worker::{DwgConversionError, ProgramWorker};
pub use ketchup_scheduler::protocol::{
    UnderlayInput, UnderlayRead, UnderlayRefusal, UnderlayWriteInput,
};
pub use ketchup_scheduler::underlay_reading::UnderlayReadError;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

/// Idle workers kept for the next request; more run only while requests overlap.
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
    with_worker(|worker| worker.evaluate(file_name, source, overrides, cancel)).unwrap_or_else(
        |error| {
            Err(ProgramError {
                code: "worker_failed",
                message: format!("programs cannot be evaluated: {error}"),
                location: None,
            })
        },
    )
}

/// [`evaluate_program`] that also ends by `deadline`, the one the checks
/// after it share.
///
/// # Errors
/// As [`evaluate_program`].
pub fn evaluate_program_by(
    file_name: &str,
    source: &str,
    overrides: &BTreeMap<String, f64>,
    deadline: CheckDeadline,
    cancel: &CancelToken,
) -> Result<Evaluated, ProgramError> {
    with_worker(|worker| worker.evaluate_by(file_name, source, overrides, deadline, cancel))
        .unwrap_or_else(|error| {
            Err(ProgramError {
                code: "worker_failed",
                message: format!("programs cannot be evaluated: {error}"),
                location: None,
            })
        })
}

/// [`ketchup_program::report`] in a program worker, under its memory limit and
/// by `deadline` (review PRG-1).
///
/// # Errors
/// `report_too_large`, `time_limit`, `memory_limit` and `worker_failed`.
pub fn report_program(
    evaluated: &Evaluated,
    deadline: CheckDeadline,
    cancel: &CancelToken,
) -> Result<Report, ProgramError> {
    with_worker(|worker| worker.report(evaluated, deadline, cancel)).unwrap_or_else(|error| {
        Err(ProgramError {
            code: "worker_failed",
            message: format!("programs cannot be checked: {error}"),
            location: None,
        })
    })
}

/// [`ketchup_program::report_before_exact`] in a program worker, like
/// [`report_program`].
///
/// # Errors
/// As [`report_program`].
pub fn report_program_before_exact(
    evaluated: &Evaluated,
    deadline: CheckDeadline,
    cancel: &CancelToken,
) -> Result<Report, ProgramError> {
    with_worker(|worker| worker.report_before_exact(evaluated, deadline, cancel)).unwrap_or_else(
        |error| {
            Err(ProgramError {
                code: "worker_failed",
                message: format!("programs cannot be checked: {error}"),
                location: None,
            })
        },
    )
}

/// The least time the checks a report left for its exact check get when it is
/// published without that check, however little of its deadline is left.
pub const PENDING_CHECKS_TIME: Duration = Duration::from_secs(5);

/// Runs the checks [`report_program_before_exact`] left out, for a report that
/// is published without the exact check that re-derives its issues and
/// relations: it then lists the same ones [`report_program`] does. Checks that
/// do not finish leave a report that says so.
pub fn complete_pending_checks(
    report: &mut Report,
    model: &ProgramModel,
    deadline: CheckDeadline,
    cancel: &CancelToken,
) {
    if !report.checks_pending() {
        return;
    }
    let deadline = if deadline.remaining() >= PENDING_CHECKS_TIME {
        deadline
    } else {
        CheckDeadline::after(PENDING_CHECKS_TIME)
    };
    match refine_report(model, &ExactShapes::default(), deadline, cancel) {
        Ok(refined) => report.apply_refined(refined),
        Err(stopped) => report.stop_pending_checks(&stopped),
    }
}

/// [`ketchup_program::refine`] in a program worker, under the limits of a
/// report and by its deadline: it repeats the checks of the report (review
/// 2026-10-15 N-4).
///
/// # Errors
/// As [`report_program`].
pub fn refine_report(
    model: &ProgramModel,
    exact: &ExactShapes,
    deadline: CheckDeadline,
    cancel: &CancelToken,
) -> Result<Refined, ProgramError> {
    with_worker(|worker| worker.refine(model, exact, deadline, cancel)).unwrap_or_else(|error| {
        Err(ProgramError {
            code: "worker_failed",
            message: format!("programs cannot be checked: {error}"),
            location: None,
        })
    })
}

/// [`ketchup_program::run`] with the evaluation and the report in one program
/// worker, both by `deadline`; `cancel` stops either.
///
/// # Errors
/// As [`evaluate_program`] and [`report_program`].
pub fn run_program(
    file_name: &str,
    source: &str,
    overrides: &BTreeMap<String, f64>,
    deadline: CheckDeadline,
    cancel: &CancelToken,
) -> Result<(Evaluated, Report), ProgramError> {
    with_worker(|worker| worker.run(file_name, source, overrides, deadline, cancel)).unwrap_or_else(
        |error| {
            Err(ProgramError {
                code: "worker_failed",
                message: format!("programs cannot be evaluated: {error}"),
                location: None,
            })
        },
    )
}

/// [`ketchup_dwg::dwg_to_dxf`] in a program worker, for DXF text of at most
/// `max_pairs` group-code pairs.
///
/// # Errors
/// The reader refused the file, its DXF has more pairs, or reading it was stopped
/// at the worker's memory or time limit.
pub fn convert_dwg(
    dwg: &[u8],
    max_pairs: usize,
    cancel: &CancelToken,
) -> Result<DwgDrawing, DwgReadError> {
    with_worker(|worker| worker.convert_dwg(dwg, max_pairs, cancel))
        .map_err(DwgReadError::Unavailable)?
        .map_err(DwgReadError::Conversion)
}

/// Reads the drawing `input` names as an underlay in a program worker, which writes its
/// lines to `input.output`; `cancel` stops it.
///
/// # Errors
/// No worker could take it, the drawing was refused, or reading it was cancelled or
/// stopped at the worker's memory limit.
pub fn read_underlay(
    input: UnderlayInput,
    cancel: &CancelToken,
) -> Result<UnderlayRead, UnderlayWorkerError> {
    with_worker(|worker| worker.read_underlay(input, cancel))
        .map_err(UnderlayWorkerError::Unavailable)?
        .map_err(UnderlayWorkerError::Read)
}

/// Writes the drawing `input` names back as DXF with its changes, in a program worker,
/// within `time`; gives the bytes written to `input.output`.
///
/// # Errors
/// No worker could take it, the drawing or its changes were refused, or writing it was
/// cancelled, ran out of time or stopped at the worker's memory limit.
pub fn write_underlay(
    input: UnderlayWriteInput,
    time: std::time::Duration,
    cancel: &CancelToken,
) -> Result<u64, UnderlayWorkerError> {
    with_worker(|worker| worker.write_underlay(input, time, cancel))
        .map_err(UnderlayWorkerError::Unavailable)?
        .map_err(UnderlayWorkerError::Read)
}

/// Why a drawing was not read as an underlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnderlayWorkerError {
    Unavailable(WorkerUnavailable),
    Read(UnderlayReadError),
}

impl fmt::Display for UnderlayWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(error) => write!(formatter, "drawings cannot be read: {error}"),
            Self::Read(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for UnderlayWorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) => Some(error),
            Self::Read(error) => Some(error),
        }
    }
}

/// Why no program worker could take a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerUnavailable {
    /// `ketchup-program-worker` is not beside the exact worker.
    NotInstalled,
    /// The worker executable is there but was refused.
    Refused(WorkerError),
}

impl fmt::Display for WorkerUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => {
                formatter.write_str("ketchup-program-worker is not installed next to Kečup")
            }
            Self::Refused(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WorkerUnavailable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotInstalled => None,
            Self::Refused(error) => Some(error),
        }
    }
}

/// Why a DWG file was not read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DwgReadError {
    Unavailable(WorkerUnavailable),
    Conversion(DwgConversionError),
}

impl fmt::Display for DwgReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(error) => write!(formatter, "DWG files cannot be read: {error}"),
            Self::Conversion(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DwgReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable(error) => Some(error),
            Self::Conversion(error) => Some(error),
        }
    }
}

/// Runs `request` on an idle worker or a new one, keeping the worker for the next.
fn with_worker<T>(request: impl FnOnce(&mut ProgramWorker) -> T) -> Result<T, WorkerUnavailable> {
    let idle = IDLE_WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop();
    let mut worker = match idle {
        Some(worker) => worker,
        None => new_worker()?,
    };
    let result = request(&mut worker);
    let mut idle = IDLE_WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if idle.len() < MAX_IDLE_WORKERS {
        idle.push(worker);
    }
    Ok(result)
}

const PROGRAM_WORKER: &str = if cfg!(windows) {
    "ketchup-program-worker.exe"
} else {
    "ketchup-program-worker"
};

/// The program worker is installed beside the exact worker.
fn new_worker() -> Result<ProgramWorker, WorkerUnavailable> {
    let executable = configured_exact_worker()
        .into_iter()
        .chain(exact_worker_candidates())
        .map(|exact| exact.with_file_name(PROGRAM_WORKER))
        .find(|path| path.is_file())
        .ok_or(WorkerUnavailable::NotInstalled)?;
    ProgramWorker::new(executable).map_err(WorkerUnavailable::Refused)
}
