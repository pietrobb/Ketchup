//! Rule programs and DWG files are read in a worker process of their own, in a
//! job with a memory limit. A program that takes all the memory or never ends,
//! or a DWG file that declares a section of hundreds of gigabytes, costs that
//! worker, not the editor window: the worker is killed on cancel and at the
//! deadline, and a failed allocation aborts only the worker.
//!
//! The worker stays between requests, so an edit pays no process start; a new
//! one starts after the last was killed or died.
//!
//! The worker is `ketchup-program-worker`, not the exact worker, so the exact
//! worker that every model evaluation starts carries neither the interpreter nor
//! the DWG reader.
pub use crate::protocol::DwgRefusal;
use crate::protocol::{
    DwgInput, Frame, MAX_REPLY_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, ProgramInput, RefineInput,
    RefineOutcome, ReportOutcome, WorkerFailure, WorkerReply, WorkerRequest, protocol_identity,
    read_frame, write_frame,
};
use crate::{ExactWorkerClient, WorkerError, exact_worker_identity, verify_exact_worker_identity};
use ketchup_dwg::{DwgDrawing, MAX_DWG_SOURCE_BYTES};
use ketchup_program::{
    CancelToken, EVALUATION_HEAP_BYTES, EVALUATION_TIME, Evaluated, ExactShapes, ProgramError,
    ProgramModel, Refined, Report,
};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ChildStderr;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Memory one program worker may commit: the interpreter's own heap limit, and
/// as much again for the library, the model and its copy on the wire.
pub const PROGRAM_WORKER_MEMORY_BYTES: usize = 2 * EVALUATION_HEAP_BYTES;

/// The interpreter looks at its clock only between statements; one builtin call
/// can run far past it, so the parent stops the worker a little later.
const DEADLINE_MARGIN: Duration = Duration::from_secs(5);

/// How long evaluating a program and every check of its model may take
/// together: the checks of the largest example program (1 854 parts) take far
/// longer than its evaluation, 1.5 s in a release build and 28 s in a debug build.
pub const PROGRAM_CHECK_TIME: Duration = Duration::from_secs(60);

/// One deadline for evaluating a program, its report, the exact check and the
/// refined report: each step gets only the time the earlier ones left, so a
/// program that is cheap to evaluate but costly to check still stops at one
/// predictable time (review 2026-10-15 S3).
#[derive(Clone, Copy, Debug)]
pub struct CheckDeadline {
    until: Instant,
    total: Duration,
}

impl CheckDeadline {
    /// [`PROGRAM_CHECK_TIME`] from now.
    #[must_use]
    pub fn start() -> Self {
        Self::after(PROGRAM_CHECK_TIME)
    }

    #[must_use]
    pub fn after(total: Duration) -> Self {
        Self {
            until: Instant::now() + total,
            total,
        }
    }

    /// Time left; zero once the deadline has passed.
    #[must_use]
    pub fn remaining(self) -> Duration {
        self.until.saturating_duration_since(Instant::now())
    }

    /// The time a request may take, or the stop when none is left.
    fn request_time(self) -> Result<Duration, Stopped> {
        let left = self.remaining();
        if left.is_zero() {
            Err(Stopped::TimeLimit(self.total))
        } else {
            Ok(left)
        }
    }

    /// A request stopped at the time left was stopped at the whole deadline.
    fn stopped(self, stopped: Stopped) -> Stopped {
        match stopped {
            Stopped::TimeLimit(_) => Stopped::TimeLimit(self.total),
            other => other,
        }
    }
}

/// How long reading one DWG file may take.
const DWG_DEADLINE: Duration = Duration::from_secs(60);

/// The DXF text of a DWG drawing comes back in one reply.
const MAX_DWG_DXF_BYTES: usize = MAX_REPLY_FRAME_BYTES - 1024 * 1024;

/// How long a dead worker's standard error may take to arrive.
const STDERR_WAIT: Duration = Duration::from_secs(3);

/// Bytes of standard error kept: enough for the allocation failure message.
const STDERR_TAIL_BYTES: usize = 4096;

struct Running {
    client: ExactWorkerClient,
    stderr: Receiver<Vec<u8>>,
}

/// Why the worker gave no answer.
enum Stopped {
    Cancelled,
    TimeLimit(Duration),
    MemoryLimit,
    Failed(WorkerError),
}

/// Why a DWG file was not read as DXF.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DwgConversionError {
    /// The DWG reader refused the file.
    Refused(DwgRefusal),
    Cancelled,
    TimeLimit(Duration),
    /// Reading needed more memory than the worker may commit.
    MemoryLimit,
    /// The worker could not be started or stopped otherwise.
    WorkerFailed(WorkerError),
}

impl fmt::Display for DwgConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(formatter),
            Self::Cancelled => formatter.write_str("reading the DWG file was cancelled"),
            Self::TimeLimit(after) => write!(
                formatter,
                "reading the DWG file took longer than {} s and was stopped",
                after.as_secs()
            ),
            Self::MemoryLimit => write!(
                formatter,
                "reading the DWG file needed more than {} GiB of memory and was stopped; the \
                 file is damaged or declares sizes it does not hold",
                PROGRAM_WORKER_MEMORY_BYTES >> 30
            ),
            Self::WorkerFailed(error) => {
                write!(
                    formatter,
                    "the process that reads DWG files stopped: {error}"
                )
            }
        }
    }
}

impl std::error::Error for DwgConversionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::WorkerFailed(error) => Some(error),
            _ => None,
        }
    }
}

pub struct ProgramWorker {
    executable: PathBuf,
    executable_sha256: String,
    deadline: Duration,
    running: Option<Running>,
}

impl ProgramWorker {
    /// Checks the worker executable; the process starts with the first evaluation.
    ///
    /// # Errors
    /// The executable is not an absolute path to a bounded file.
    pub fn new(executable: impl AsRef<Path>) -> Result<Self, WorkerError> {
        let (executable, executable_sha256) = exact_worker_identity(executable.as_ref())?;
        Ok(Self {
            executable,
            executable_sha256,
            deadline: EVALUATION_TIME + DEADLINE_MARGIN,
            running: None,
        })
    }

    /// Stops an evaluation after `deadline` instead of its own limit; tests use
    /// it to reach the deadline quickly.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Process id of the worker that evaluates the next program, if one runs.
    pub fn process_id(&mut self) -> Option<u32> {
        let running = self.running.as_mut()?;
        running.is_alive().then(|| running.client.child.id())
    }

    /// Evaluates like [`ketchup_program::evaluate_cancellable`], in the worker.
    ///
    /// # Errors
    /// As the program itself, plus `cancelled`, `time_limit`, `memory_limit` and
    /// `worker_failed` when the worker was stopped or died.
    pub fn evaluate(
        &mut self,
        file_name: &str,
        source: &str,
        overrides: &BTreeMap<String, f64>,
        cancel: &CancelToken,
    ) -> Result<Evaluated, ProgramError> {
        self.evaluate_within(file_name, source, overrides, self.deadline, cancel)
    }

    fn evaluate_within(
        &mut self,
        file_name: &str,
        source: &str,
        overrides: &BTreeMap<String, f64>,
        time: Duration,
        cancel: &CancelToken,
    ) -> Result<Evaluated, ProgramError> {
        let input = ProgramInput {
            file_name: file_name.to_owned(),
            source: source.to_owned(),
            overrides: overrides.clone(),
        };
        self.call(cancel, |client| {
            client.evaluate_program(input, cancel.flag(), time)
        })
        .map_err(Stopped::program_error)?
        .into()
    }

    /// Evaluates and reports like [`ketchup_program::run`], in the worker: the
    /// evaluation keeps its own limit, and both end by `deadline`.
    ///
    /// # Errors
    /// As [`Self::evaluate`] and [`Self::report`].
    pub fn run(
        &mut self,
        file_name: &str,
        source: &str,
        overrides: &BTreeMap<String, f64>,
        deadline: CheckDeadline,
        cancel: &CancelToken,
    ) -> Result<(Evaluated, Report), ProgramError> {
        let time = self
            .deadline
            .min(deadline.request_time().map_err(Stopped::program_error)?);
        let evaluated = self.evaluate_within(file_name, source, overrides, time, cancel)?;
        let report = self.report(&evaluated, deadline, cancel)?;
        Ok((evaluated, report))
    }

    /// [`ketchup_program::report`] of `evaluated` in the worker, under the
    /// memory limit of an evaluation, by `deadline`: on a large model the checks
    /// cost far more than the evaluation and have no bound of their own.
    ///
    /// # Errors
    /// `report_too_large`, or `cancelled`, `time_limit`, `memory_limit` and
    /// `worker_failed` when the worker was stopped or died.
    pub fn report(
        &mut self,
        evaluated: &Evaluated,
        deadline: CheckDeadline,
        cancel: &CancelToken,
    ) -> Result<Report, ProgramError> {
        let input = evaluated.clone().into();
        let outcome = self
            .call_by(deadline, cancel, |client, time| {
                client.report_program(input, cancel.flag(), time)
            })
            .map_err(Stopped::report_error)?;
        match outcome {
            ReportOutcome::Reported(report) => Ok(report.with_part_sources(evaluated)),
            ReportOutcome::Failed(error) => Err(error),
        }
    }

    /// [`ketchup_program::refine`] in the worker, under the memory limit of a
    /// report and by the deadline of the report it refines.
    ///
    /// # Errors
    /// As [`Self::report`].
    pub fn refine(
        &mut self,
        model: &ProgramModel,
        exact: &ExactShapes,
        deadline: CheckDeadline,
        cancel: &CancelToken,
    ) -> Result<Refined, ProgramError> {
        let input = RefineInput {
            model: model.clone(),
            exact: exact.clone(),
        };
        let outcome = self
            .call_by(deadline, cancel, |client, time| {
                client.refine_report(input, cancel.flag(), time)
            })
            .map_err(Stopped::report_error)?;
        match outcome {
            RefineOutcome::Refined(refined) => Ok(*refined),
            RefineOutcome::Failed(error) => Err(error),
        }
    }

    /// [`ketchup_dwg::dwg_to_dxf`] in the worker, for a file of at most
    /// [`MAX_DWG_SOURCE_BYTES`] and DXF text of at most `max_dxf_bytes`.
    ///
    /// # Errors
    /// The reader refused the file, or the worker was stopped or died.
    pub fn convert_dwg(
        &mut self,
        dwg: &[u8],
        max_dxf_bytes: usize,
        cancel: &CancelToken,
    ) -> Result<DwgDrawing, DwgConversionError> {
        // Refused here too: the request must fit its frame.
        if dwg.len() > MAX_DWG_SOURCE_BYTES {
            return Err(DwgConversionError::Refused(DwgRefusal::SourceTooLarge {
                bytes: dwg.len(),
                limit: MAX_DWG_SOURCE_BYTES,
            }));
        }
        let input = DwgInput {
            dwg: dwg.to_vec(),
            max_dxf_bytes: max_dxf_bytes.min(MAX_DWG_DXF_BYTES),
        };
        let outcome = self
            .call(cancel, |client| {
                client.convert_dwg(input, cancel.flag(), DWG_DEADLINE)
            })
            .map_err(Stopped::dwg_error)?;
        Result::from(outcome).map_err(DwgConversionError::Refused)
    }

    /// [`Self::call`] with the time `deadline` has left.
    fn call_by<T>(
        &mut self,
        deadline: CheckDeadline,
        cancel: &CancelToken,
        request: impl FnOnce(&mut ExactWorkerClient, Duration) -> Result<T, WorkerError>,
    ) -> Result<T, Stopped> {
        let time = deadline.request_time()?;
        self.call(cancel, |client| request(client, time))
            .map_err(|stopped| deadline.stopped(stopped))
    }

    /// Sends one request to the running worker, starting one if none runs.
    fn call<T>(
        &mut self,
        cancel: &CancelToken,
        request: impl FnOnce(&mut ExactWorkerClient) -> Result<T, WorkerError>,
    ) -> Result<T, Stopped> {
        if cancel.is_cancelled() {
            return Err(Stopped::Cancelled);
        }
        let alive = self.running.as_mut().is_some_and(Running::is_alive);
        let mut running = match self.running.take() {
            Some(running) if alive => running,
            _ => self.spawn(cancel)?,
        };
        match request(&mut running.client) {
            Ok(reply) => {
                self.running = Some(running);
                Ok(reply)
            }
            // The client has already killed the worker; it is dropped here.
            Err(error) => Err(stopped(error, &running.stderr)),
        }
    }

    fn spawn(&self, cancel: &CancelToken) -> Result<Running, Stopped> {
        let executable = verify_exact_worker_identity(&self.executable, &self.executable_sha256)
            .map_err(Stopped::Failed)?;
        let (mut client, stderr) =
            ExactWorkerClient::spawn_guarded_with(executable, Some(PROGRAM_WORKER_MEMORY_BYTES))
                .map_err(Stopped::Failed)?;
        let stderr = stderr_tail(stderr);
        client
            .ping_with_cancellation(cancel.flag())
            .map_err(|error| stopped(error, &stderr))?;
        Ok(Running { client, stderr })
    }
}

impl Running {
    fn is_alive(&mut self) -> bool {
        !self.client.terminated && matches!(self.client.child.try_wait(), Ok(None))
    }
}

/// The worker process: answers the handshake, evaluates programs and reads DWG
/// files until its input closes. The parent kills it to cancel or stop it, so
/// it needs no token.
pub fn run_stdio() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        let reply = match read_frame::<WorkerRequest>(&mut stdin, MAX_REQUEST_FRAME_BYTES) {
            Frame::Message(WorkerRequest::Hello) => WorkerReply::Hello {
                protocol: protocol_identity().to_owned(),
            },
            Frame::Message(WorkerRequest::EvaluateProgram(input)) => {
                WorkerReply::Program(Box::new(
                    ketchup_program::evaluate(&input.file_name, &input.source, &input.overrides)
                        .into(),
                ))
            }
            Frame::Message(WorkerRequest::ReportProgram(input)) => {
                let report = ketchup_program::report(&Evaluated::from(*input));
                WorkerReply::Report(Box::new(ReportOutcome::Reported(Box::new(report))))
            }
            Frame::Message(WorkerRequest::RefineReport(input)) => {
                let refined = ketchup_program::refine(&input.model, &input.exact);
                WorkerReply::Refined(Box::new(RefineOutcome::Refined(Box::new(refined))))
            }
            Frame::Message(WorkerRequest::ConvertDwg(input)) => WorkerReply::Dwg(Box::new(
                ketchup_dwg::dwg_to_dxf(
                    &input.dwg,
                    MAX_DWG_SOURCE_BYTES,
                    input.max_dxf_bytes.min(MAX_DWG_DXF_BYTES),
                )
                .into(),
            )),
            Frame::Message(_) | Frame::Malformed(_) => {
                WorkerReply::Failure(WorkerFailure::invalid_request())
            }
            Frame::Closed | Frame::TooLarge | Frame::Transport(_) => break,
        };
        let written = match write_frame(&mut stdout, &reply, MAX_REPLY_FRAME_BYTES) {
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                write_frame(&mut stdout, &too_large(&reply), MAX_REPLY_FRAME_BYTES)
            }
            written => written,
        };
        if written.is_err() {
            break;
        }
    }
}

/// What the worker answers instead of a reply larger than its frame.
fn too_large(reply: &WorkerReply) -> WorkerReply {
    let error = || ProgramError {
        code: "report_too_large",
        message: format!(
            "the program's model has more issues and relations than one report of {} MiB \
             holds; fix the overlapping parts it reports in smaller groups",
            MAX_REPLY_FRAME_BYTES >> 20
        ),
        location: None,
    };
    match reply {
        WorkerReply::Report(_) => WorkerReply::Report(Box::new(ReportOutcome::Failed(error()))),
        WorkerReply::Refined(_) => WorkerReply::Refined(Box::new(RefineOutcome::Failed(error()))),
        _ => WorkerReply::Failure(WorkerFailure::code("resource_limit")),
    }
}

/// Collects the last bytes of the worker's standard error; they arrive when it exits.
fn stderr_tail(stderr: Option<ChildStderr>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    if let Some(mut stderr) = stderr {
        let _ = std::thread::spawn(move || {
            let mut tail = Vec::new();
            let mut buffer = [0_u8; 1024];
            while let Ok(read) = stderr.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                tail.extend_from_slice(&buffer[..read]);
                let excess = tail.len().saturating_sub(STDERR_TAIL_BYTES);
                tail.drain(..excess);
            }
            let _ = sender.send(tail);
        });
    }
    receiver
}

fn stopped(error: WorkerError, stderr: &Receiver<Vec<u8>>) -> Stopped {
    match error {
        WorkerError::Cancelled => Stopped::Cancelled,
        WorkerError::RequestTimedOut(after) => Stopped::TimeLimit(after),
        _ => {
            let tail = stderr.recv_timeout(STDERR_WAIT).unwrap_or_default();
            if String::from_utf8_lossy(&tail).contains("memory allocation of") {
                Stopped::MemoryLimit
            } else {
                Stopped::Failed(error)
            }
        }
    }
}

impl Stopped {
    fn program_error(self) -> ProgramError {
        let (code, message) = match self {
            Self::Cancelled => (
                "cancelled",
                "program evaluation cancelled: a newer program replaced it".to_owned(),
            ),
            Self::TimeLimit(after) => (
                "time_limit",
                format!(
                    "the program ran longer than {} s and was stopped; shorten loops or split the calculation",
                    after.as_secs()
                ),
            ),
            Self::MemoryLimit => (
                "memory_limit",
                format!(
                    "the program needed more than {} GiB of memory and was stopped; build smaller strings, lists or dictionaries",
                    PROGRAM_WORKER_MEMORY_BYTES >> 30
                ),
            ),
            Self::Failed(error) => (
                "worker_failed",
                format!("the process that evaluates programs stopped: {error}"),
            ),
        };
        ProgramError {
            code,
            message,
            location: None,
        }
    }

    fn report_error(self) -> ProgramError {
        let message = match &self {
            Self::TimeLimit(after) => format!(
                "checking the program's model (collisions, relations, loads) took longer than {} s \
                 and was stopped; too many parts overlap or touch one another",
                after.as_secs()
            ),
            Self::MemoryLimit => format!(
                "checking the program's model (collisions, relations, loads) needed more than {} GiB \
                 of memory and was stopped; too many parts overlap or touch one another",
                PROGRAM_WORKER_MEMORY_BYTES >> 30
            ),
            Self::Cancelled | Self::Failed(_) => return self.program_error(),
        };
        ProgramError {
            message,
            ..self.program_error()
        }
    }

    fn dwg_error(self) -> DwgConversionError {
        match self {
            Self::Cancelled => DwgConversionError::Cancelled,
            Self::TimeLimit(after) => DwgConversionError::TimeLimit(after),
            Self::MemoryLimit => DwgConversionError::MemoryLimit,
            Self::Failed(error) => DwgConversionError::WorkerFailed(error),
        }
    }
}
