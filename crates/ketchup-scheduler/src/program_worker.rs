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
    ANY_FRAME_BYTES, DwgInput, DwgOutcome, Frame, MAX_REPLY_FRAME_BYTES, ProgramInput, RefineInput,
    RefineOutcome, ReportOutcome, UnderlayInput, UnderlayOutcome, UnderlayRead, WorkerFailure,
    WorkerReply, WorkerRequest, protocol_identity, read_frame, write_frame,
};
use crate::underlay_reading::{UnderlayReadError, read_underlay};
use crate::{ExactWorkerClient, WorkerError, exact_worker_identity, verify_exact_worker_identity};
use ketchup_dwg::DwgDrawing;
use ketchup_model::graph::sha256_hex;
use ketchup_program::{
    CancelToken, EVALUATION_HEAP_BYTES, EVALUATION_TIME, Evaluated, ExactShapes, ProgramError,
    ProgramModel, Refined, Report,
};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
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

/// Reading one DWG file, or one drawing as an underlay, has no deadline: it ends, is
/// cancelled by the user, or stops at the worker's memory limit. A century stands in
/// for "never" where the worker client wants a time.
const UNTIL_CANCELLED: Duration = Duration::from_secs(100 * 365 * 24 * 60 * 60);

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
    /// Reading needed more memory than the worker may commit.
    MemoryLimit,
    /// The worker could not be started or stopped otherwise.
    WorkerFailed(WorkerError),
    /// No directory could be made for the DXF text the worker writes.
    OutputDirectory(std::io::ErrorKind),
    /// The DXF text the worker wrote could not be read back, for example for lack of memory.
    Output(WorkerError),
    /// The DXF text read back is not the text the worker wrote.
    OutputDigestMismatch,
}

impl fmt::Display for DwgConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(formatter),
            Self::Cancelled => formatter.write_str("reading the DWG file was cancelled"),
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
            Self::Output(error) => {
                write!(
                    formatter,
                    "the DXF of the DWG drawing could not be read back: {error}"
                )
            }
            Self::OutputDirectory(kind) => write!(
                formatter,
                "no directory could be made for the DXF of the DWG drawing: {kind}"
            ),
            Self::OutputDigestMismatch => {
                formatter.write_str("the DXF file does not match the worker's digest")
            }
        }
    }
}

impl std::error::Error for DwgConversionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::WorkerFailed(error) | Self::Output(error) => Some(error),
            _ => None,
        }
    }
}

pub struct ProgramWorker {
    executable: PathBuf,
    executable_sha256: String,
    deadline: Duration,
    running: Option<ExactWorkerClient>,
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
        running.is_alive().then(|| running.child.id())
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

    /// [`Self::evaluate`] that also ends by `deadline`, the one its checks share.
    ///
    /// # Errors
    /// As [`Self::evaluate`].
    pub fn evaluate_by(
        &mut self,
        file_name: &str,
        source: &str,
        overrides: &BTreeMap<String, f64>,
        deadline: CheckDeadline,
        cancel: &CancelToken,
    ) -> Result<Evaluated, ProgramError> {
        let time = self
            .deadline
            .min(deadline.request_time().map_err(Stopped::program_error)?);
        self.evaluate_within(file_name, source, overrides, time, cancel)
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
        let evaluated = self.evaluate_by(file_name, source, overrides, deadline, cancel)?;
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

    /// [`ketchup_dwg::dwg_to_dxf`] in the worker, for DXF text of any size: the worker
    /// writes it to a file of its own, read back whole here.
    ///
    /// # Errors
    /// The reader refused the file, the worker was stopped or died, or its DXF
    /// text could not be read back.
    pub fn convert_dwg(
        &mut self,
        dwg: &[u8],
        cancel: &CancelToken,
    ) -> Result<DwgDrawing, DwgConversionError> {
        let directory = tempfile::tempdir()
            .map_err(|error| DwgConversionError::OutputDirectory(error.kind()))?;
        let output = directory.path().join("drawing.dxf");
        let input = DwgInput {
            dwg: dwg.to_vec(),
            output: output.clone(),
        };
        let outcome = self
            .call(cancel, |client| {
                client.convert_dwg(input, cancel.flag(), UNTIL_CANCELLED)
            })
            .map_err(Stopped::dwg_error)?;
        let (dxf_sha256, version, skipped) = match outcome {
            DwgOutcome::Converted {
                dxf_sha256,
                version,
                skipped,
            } => (dxf_sha256, version, skipped),
            DwgOutcome::Refused(refusal) => return Err(DwgConversionError::Refused(refusal)),
        };
        let dxf = crate::read_regular_file(&output, "DXF of the DWG drawing")
            .map_err(DwgConversionError::Output)?;
        if sha256_hex(&dxf) != dxf_sha256 {
            return Err(DwgConversionError::OutputDigestMismatch);
        }
        Ok(DwgDrawing {
            dxf,
            version,
            skipped: skipped
                .into_iter()
                .map(|(record_type, count)| ketchup_dwg::SkippedRecords { record_type, count })
                .collect(),
        })
    }

    /// Reads the drawing `input` names as an underlay in the worker, which writes its
    /// lines to `input.output`.
    ///
    /// # Errors
    /// The drawing was refused, or the worker was stopped or died.
    pub fn read_underlay(
        &mut self,
        input: UnderlayInput,
        cancel: &CancelToken,
    ) -> Result<UnderlayRead, UnderlayReadError> {
        let outcome = self
            .call(cancel, |client| {
                client.read_underlay(input, cancel.flag(), UNTIL_CANCELLED)
            })
            .map_err(Stopped::underlay_error)?;
        match outcome {
            UnderlayOutcome::Read(read) => Ok(*read),
            UnderlayOutcome::Refused(refusal) => Err(UnderlayReadError::Refused(refusal)),
        }
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
        let alive = self
            .running
            .as_mut()
            .is_some_and(ExactWorkerClient::is_alive);
        let mut running = match self.running.take() {
            Some(running) if alive => running,
            _ => self.spawn(cancel)?,
        };
        match request(&mut running) {
            Ok(reply) => {
                self.running = Some(running);
                Ok(reply)
            }
            // The client has already killed the worker; it is dropped here.
            Err(error) => Err(stopped(error)),
        }
    }

    fn spawn(&self, cancel: &CancelToken) -> Result<ExactWorkerClient, Stopped> {
        let executable = verify_exact_worker_identity(&self.executable, &self.executable_sha256)
            .map_err(Stopped::Failed)?;
        let mut client =
            ExactWorkerClient::spawn_guarded_with(executable, PROGRAM_WORKER_MEMORY_BYTES)
                .map_err(Stopped::Failed)?;
        client
            .ping_with_cancellation(cancel.flag())
            .map_err(stopped)?;
        Ok(client)
    }
}

/// The worker process: answers the handshake, evaluates programs and reads DWG
/// files until its input closes. The parent kills it to cancel or stop it, so
/// it needs no token.
pub fn run_stdio() {
    crate::child_process::exit_with_parent();
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        // Any frame its length can declare: the job's memory limit bounds what this holds.
        let reply = match read_frame::<WorkerRequest>(&mut stdin, ANY_FRAME_BYTES) {
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
            Frame::Message(WorkerRequest::ConvertDwg(input)) => {
                WorkerReply::Dwg(Box::new(DwgOutcome::convert(&input)))
            }
            Frame::Message(WorkerRequest::ReadUnderlay(input)) => {
                WorkerReply::Underlay(Box::new(read_underlay(&input)))
            }
            Frame::Message(_) | Frame::Malformed(_) => {
                WorkerReply::Failure(WorkerFailure::invalid_request())
            }
            Frame::Closed | Frame::TooLarge | Frame::Transport(_) => break,
        };
        if write_reply(&mut stdout, &reply, MAX_REPLY_FRAME_BYTES).is_err() {
            break;
        }
    }
}

/// Writes `reply`; a report or refined list larger than `max_bytes` goes with
/// only its first relations, halved until it fits, and only then with fewer
/// issues, halved as well; any other too large reply goes as its failure.
fn write_reply(
    out: &mut impl std::io::Write,
    reply: &WorkerReply,
    max_bytes: usize,
) -> std::io::Result<()> {
    let too_large = |error: &std::io::Error| error.kind() == std::io::ErrorKind::InvalidData;
    match write_frame(out, reply, max_bytes) {
        Err(error) if too_large(&error) => {}
        written => return written,
    }
    let (issues, relations) = listed(reply);
    let halved =
        |count: usize| std::iter::successors(Some(count), |k| (*k > 0).then(|| k / 2)).skip(1);
    let attempts = halved(relations)
        .map(|kept| (issues, kept))
        .chain(halved(issues).map(|kept| (kept, 0)));
    for (issues, relations) in attempts {
        let Some(first) = first_listed(reply, issues, relations) else {
            break;
        };
        match write_frame(out, &first, max_bytes) {
            Err(error) if too_large(&error) => {}
            written => return written,
        }
    }
    write_frame(out, &failed_too_large(reply), max_bytes)
}

/// How many issues and relations a report or refined list holds.
fn listed(reply: &WorkerReply) -> (usize, usize) {
    match reply {
        WorkerReply::Report(outcome) => match outcome.as_ref() {
            ReportOutcome::Reported(report) => (report.issues.len(), report.relations.len()),
            ReportOutcome::Failed(_) => (0, 0),
        },
        WorkerReply::Refined(outcome) => match outcome.as_ref() {
            RefineOutcome::Refined(refined) => (refined.issues.len(), refined.relations.len()),
            RefineOutcome::Failed(_) => (0, 0),
        },
        _ => (0, 0),
    }
}

fn first_listed(reply: &WorkerReply, issues: usize, relations: usize) -> Option<WorkerReply> {
    match reply {
        WorkerReply::Report(outcome) => match outcome.as_ref() {
            ReportOutcome::Reported(report) => Some(WorkerReply::Report(Box::new(
                ReportOutcome::Reported(Box::new(report.first(issues, relations))),
            ))),
            ReportOutcome::Failed(_) => None,
        },
        WorkerReply::Refined(outcome) => match outcome.as_ref() {
            RefineOutcome::Refined(refined) => Some(WorkerReply::Refined(Box::new(
                RefineOutcome::Refined(Box::new(refined.first(issues, relations))),
            ))),
            RefineOutcome::Failed(_) => None,
        },
        _ => None,
    }
}

/// What the worker answers instead of a reply larger than its frame.
fn failed_too_large(reply: &WorkerReply) -> WorkerReply {
    let error = || ProgramError {
        code: "report_too_large",
        message: format!(
            "the program's report does not fit one reply of {} MiB even without its issues \
             and relations; check fewer parts at a time",
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

fn stopped(error: WorkerError) -> Stopped {
    match error {
        WorkerError::Cancelled => Stopped::Cancelled,
        WorkerError::RequestTimedOut(after) => Stopped::TimeLimit(after),
        WorkerError::MemoryLimit { .. } => Stopped::MemoryLimit,
        error => Stopped::Failed(error),
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
            Self::TimeLimit(after) => {
                DwgConversionError::WorkerFailed(WorkerError::RequestTimedOut(after))
            }
            Self::MemoryLimit => DwgConversionError::MemoryLimit,
            Self::Failed(error) => DwgConversionError::WorkerFailed(error),
        }
    }

    fn underlay_error(self) -> UnderlayReadError {
        match self {
            Self::Cancelled => UnderlayReadError::Cancelled,
            Self::TimeLimit(after) => {
                UnderlayReadError::WorkerFailed(WorkerError::RequestTimedOut(after))
            }
            Self::MemoryLimit => UnderlayReadError::MemoryLimit,
            Self::Failed(error) => UnderlayReadError::WorkerFailed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_program::{Issue, REPORT_TRUNCATED, Severity};

    /// Thirty boxes that all overlap: 435 collisions.
    fn overlapping() -> Evaluated {
        let source =
            "for i in range(30):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n";
        ketchup_program::evaluate("test.star", source, &BTreeMap::new()).unwrap()
    }

    /// `reply` as written to a frame of a third of its size, and read back.
    fn written_in_a_third(reply: &WorkerReply) -> WorkerReply {
        let max_bytes = crate::protocol::encode_frame(reply, usize::MAX)
            .unwrap()
            .len()
            / 3;
        let mut out = Vec::new();
        write_reply(&mut out, reply, max_bytes).unwrap();
        match read_frame(&mut out.as_slice(), max_bytes) {
            Frame::Message(reply) => reply,
            other => panic!("{other:?}"),
        }
    }

    fn assert_names_the_rest(issues: &[Issue], whole: usize) {
        let notes: Vec<_> = issues
            .iter()
            .filter(|issue| issue.kind == REPORT_TRUNCATED)
            .collect();
        assert_eq!(notes.len(), 1, "{issues:?}");
        assert_eq!(
            notes[0].severity,
            Severity::Error,
            "collisions were left out"
        );
        assert!(
            notes[0].message.contains(&format!("of its {whole} issues")),
            "{}",
            notes[0].message
        );
    }

    /// A report too large for one reply is no longer refused whole: it lists
    /// its first issues and relations and says how many it leaves out (PRG-1).
    #[test]
    fn a_report_past_its_reply_lists_its_first_issues_and_names_the_rest() {
        let whole = ketchup_program::report(&overlapping());
        assert!(
            !whole.ok && whole.issues.len() > 400,
            "{}",
            whole.issues.len()
        );
        let reply = WorkerReply::Report(Box::new(ReportOutcome::Reported(Box::new(whole.clone()))));
        let WorkerReply::Report(outcome) = written_in_a_third(&reply) else {
            panic!("not a report");
        };
        let ReportOutcome::Reported(first) = *outcome else {
            panic!("{outcome:?}");
        };
        assert!(first.issues.len() < whole.issues.len());
        assert!(first.relations.len() < whole.relations.len());
        assert!(!first.ok);
        assert_eq!(first.params, whole.params);
        assert_eq!(first.bom, whole.bom);
        assert_names_the_rest(&first.issues, whole.issues.len());
    }

    /// D-3 (review 2026-10-17): relations go first, so a report too large only
    /// for its relations keeps every issue, and it counts the whole report.
    #[test]
    fn a_report_too_large_for_its_relations_leaves_out_relations_before_issues() {
        let mut whole = ketchup_program::report(&overlapping());
        let few = whole.issues[..3].to_vec();
        let relations = whole.relations.clone();
        whole.set_issues(few.clone());
        whole.relations = relations;
        assert!(whole.relations.len() > 400, "{}", whole.relations.len());
        let reply = WorkerReply::Report(Box::new(ReportOutcome::Reported(Box::new(whole.clone()))));
        let WorkerReply::Report(outcome) = written_in_a_third(&reply) else {
            panic!("not a report");
        };
        let ReportOutcome::Reported(first) = *outcome else {
            panic!("{outcome:?}");
        };
        assert!(first.relations.len() < whole.relations.len());
        for issue in &few {
            assert!(first.issues.contains(issue), "{issue:?} left out");
        }
        assert_eq!(first.issues.len(), few.len() + 1, "{:?}", first.issues);
    }

    #[test]
    fn a_refined_list_past_its_reply_keeps_its_first_issues_too() {
        let evaluated = overlapping();
        let whole = ketchup_program::refine(&evaluated.model, &ExactShapes::default());
        let reply = WorkerReply::Refined(Box::new(RefineOutcome::Refined(Box::new(whole.clone()))));
        let WorkerReply::Refined(outcome) = written_in_a_third(&reply) else {
            panic!("not a refined list");
        };
        let RefineOutcome::Refined(first) = *outcome else {
            panic!("{outcome:?}");
        };
        assert!(first.issues.len() < whole.issues.len());
        assert_names_the_rest(&first.issues, whole.issues.len());
    }
}
