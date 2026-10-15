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
    DwgInput, Frame, MAX_REPLY_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, ProgramInput, ReportOutcome,
    WorkerFailure, WorkerReply, WorkerRequest, protocol_identity, read_frame, write_frame,
};
use crate::{ExactWorkerClient, WorkerError, exact_worker_identity, verify_exact_worker_identity};
use ketchup_dwg::{DwgDrawing, MAX_DWG_SOURCE_BYTES};
use ketchup_program::{
    CancelToken, EVALUATION_HEAP_BYTES, EVALUATION_TIME, Evaluated, ProgramError, Report,
};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ChildStderr;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// Memory one program worker may commit: the interpreter's own heap limit, and
/// as much again for the library, the model and its copy on the wire.
pub const PROGRAM_WORKER_MEMORY_BYTES: usize = 2 * EVALUATION_HEAP_BYTES;

/// The interpreter looks at its clock only between statements; one builtin call
/// can run far past it, so the parent stops the worker a little later.
const DEADLINE_MARGIN: Duration = Duration::from_secs(5);

/// How long checking one evaluated model may take: the checks of the largest
/// example program (1 854 parts) take far longer than its evaluation, 1.5 s in
/// a release build and 28 s in a debug build.
const REPORT_DEADLINE: Duration = Duration::from_secs(60);

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
    report_deadline: Duration,
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
            report_deadline: REPORT_DEADLINE,
            running: None,
        })
    }

    /// Stops an evaluation and a report after `deadline` instead of their own
    /// limits; tests use it to reach the deadline quickly.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self.report_deadline = deadline;
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
        let input = ProgramInput {
            file_name: file_name.to_owned(),
            source: source.to_owned(),
            overrides: overrides.clone(),
        };
        let deadline = self.deadline;
        self.call(cancel, |client| {
            client.evaluate_program(input, cancel.flag(), deadline)
        })
        .map_err(Stopped::program_error)?
        .into()
    }

    /// [`ketchup_program::report`] of `evaluated` in the worker, under the
    /// memory limit of an evaluation and a deadline of its own: on a large model
    /// the checks cost far more than the evaluation and have no bound of their own.
    ///
    /// # Errors
    /// `report_too_large`, or `cancelled`, `time_limit`, `memory_limit` and
    /// `worker_failed` when the worker was stopped or died.
    pub fn report(
        &mut self,
        evaluated: &Evaluated,
        cancel: &CancelToken,
    ) -> Result<Report, ProgramError> {
        let input = evaluated.clone().into();
        let deadline = self.report_deadline;
        let outcome = self
            .call(cancel, |client| {
                client.report_program(input, cancel.flag(), deadline)
            })
            .map_err(Stopped::report_error)?;
        match outcome {
            ReportOutcome::Reported(report) => Ok(report.with_part_sources(evaluated)),
            ReportOutcome::Failed(error) => Err(error),
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
    match reply {
        WorkerReply::Report(_) => {
            WorkerReply::Report(Box::new(ReportOutcome::Failed(ProgramError {
                code: "report_too_large",
                message: format!(
                    "the program's model has more issues and relations than one report of {} MiB \
                 holds; fix the overlapping parts it reports in smaller groups",
                    MAX_REPLY_FRAME_BYTES >> 20
                ),
                location: None,
            })))
        }
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
