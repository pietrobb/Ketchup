//! Rule programs run in a worker process of their own, in a job with a memory
//! limit. A program that takes all the memory or never ends costs that worker,
//! not the editor window: the worker is killed on cancel and at the deadline,
//! and a failed allocation aborts only the worker.
//!
//! The worker stays between evaluations, so an edit pays no process start; a new
//! one starts after the last was killed or died.
//!
//! The worker is `ketchup-program-worker`, not the exact worker, so the exact
//! worker that every model evaluation starts does not carry the interpreter.
use crate::protocol::{
    Frame, MAX_REPLY_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, ProgramInput, WorkerFailure,
    WorkerReply, WorkerRequest, protocol_identity, read_frame, write_frame,
};
use crate::{ExactWorkerClient, WorkerError, exact_worker_identity, verify_exact_worker_identity};
use ketchup_program::{
    CancelToken, EVALUATION_HEAP_BYTES, EVALUATION_TIME, Evaluated, ProgramError,
};
use std::collections::BTreeMap;
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

/// How long a dead worker's standard error may take to arrive.
const STDERR_WAIT: Duration = Duration::from_secs(3);

/// Bytes of standard error kept: enough for the allocation failure message.
const STDERR_TAIL_BYTES: usize = 4096;

struct Running {
    client: ExactWorkerClient,
    stderr: Receiver<Vec<u8>>,
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

    /// Stops an evaluation after `deadline` instead of the interpreter's limit
    /// plus a margin; tests use it to reach the deadline quickly.
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
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let alive = self.running.as_mut().is_some_and(Running::is_alive);
        let mut running = match self.running.take() {
            Some(running) if alive => running,
            _ => self.spawn(cancel)?,
        };
        let input = ProgramInput {
            file_name: file_name.to_owned(),
            source: source.to_owned(),
            overrides: overrides.clone(),
        };
        match running
            .client
            .evaluate_program(input, cancel.flag(), self.deadline)
        {
            Ok(outcome) => {
                self.running = Some(running);
                outcome.into()
            }
            // The client has already killed the worker; it is dropped here.
            Err(error) => Err(stopped(&error, &running.stderr)),
        }
    }

    fn spawn(&self, cancel: &CancelToken) -> Result<Running, ProgramError> {
        let failed = |error: WorkerError| worker_failed(&error);
        let executable = verify_exact_worker_identity(&self.executable, &self.executable_sha256)
            .map_err(failed)?;
        let (mut client, stderr) =
            ExactWorkerClient::spawn_guarded_with(executable, Some(PROGRAM_WORKER_MEMORY_BYTES))
                .map_err(failed)?;
        let stderr = stderr_tail(stderr);
        client
            .ping_with_cancellation(cancel.flag())
            .map_err(|error| stopped(&error, &stderr))?;
        Ok(Running { client, stderr })
    }
}

impl Running {
    fn is_alive(&mut self) -> bool {
        !self.client.terminated && matches!(self.client.child.try_wait(), Ok(None))
    }
}

/// The worker process: answers the handshake and evaluates programs until its
/// input closes. The parent kills it to cancel or stop it, so it needs no token.
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
            Frame::Message(_) | Frame::Malformed(_) => {
                WorkerReply::Failure(WorkerFailure::invalid_request())
            }
            Frame::Closed | Frame::TooLarge | Frame::Transport(_) => break,
        };
        let written = match write_frame(&mut stdout, &reply, MAX_REPLY_FRAME_BYTES) {
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => write_frame(
                &mut stdout,
                &WorkerReply::Failure(WorkerFailure::code("resource_limit")),
                MAX_REPLY_FRAME_BYTES,
            ),
            written => written,
        };
        if written.is_err() {
            break;
        }
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

fn stopped(error: &WorkerError, stderr: &Receiver<Vec<u8>>) -> ProgramError {
    match error {
        WorkerError::Cancelled => cancelled(),
        WorkerError::RequestTimedOut(after) => ProgramError {
            code: "time_limit",
            message: format!(
                "the program ran longer than {} s and was stopped; shorten loops or split the calculation",
                after.as_secs()
            ),
            location: None,
        },
        _ => {
            let tail = stderr.recv_timeout(STDERR_WAIT).unwrap_or_default();
            if String::from_utf8_lossy(&tail).contains("memory allocation of") {
                ProgramError {
                    code: "memory_limit",
                    message: format!(
                        "the program needed more than {} GiB of memory and was stopped; build smaller strings, lists or dictionaries",
                        PROGRAM_WORKER_MEMORY_BYTES >> 30
                    ),
                    location: None,
                }
            } else {
                worker_failed(error)
            }
        }
    }
}

fn cancelled() -> ProgramError {
    ProgramError {
        code: "cancelled",
        message: "program evaluation cancelled: a newer program replaced it".to_owned(),
        location: None,
    }
}

fn worker_failed(error: &WorkerError) -> ProgramError {
    ProgramError {
        code: "worker_failed",
        message: format!("the process that evaluates programs stopped: {error}"),
        location: None,
    }
}
