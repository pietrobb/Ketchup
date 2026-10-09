//! `program action=check`: plans a whole program or a patch on a copy of the
//! document, builds its parts there and checks them exactly as `apply` would,
//! without publishing anything. Runs off the UI thread like `apply`.
#[cfg(test)]
#[path = "program_dry_run_tests.rs"]
mod tests;
use super::program_check::{PROGRAM_EXACT_TIMEOUT, ProgramCheckJob};
use super::*;
use ketchup_application::{RuleProgramApplyError, RuleProgramChange, SessionError};
use ketchup_model::document::{ProposalContext, RuleProgramSource};
use ketchup_model::persistence::ContainerData;

pub(super) struct DryRunJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    before: Stamp,
    started: Instant,
    context: CheckContext,
    receiver: mpsc::Receiver<Result<Checked, RuleProgramApplyError>>,
}

/// What the answer needs from the window, taken when the check starts.
struct CheckContext {
    undo_steps: usize,
    owns_content: bool,
    source_diff: Value,
}

struct Checked {
    change: &'static str,
    report: ketchup_program::Report,
    model: ketchup_program::ProgramModel,
    before: Snapshot,
    after: Snapshot,
    exact: Option<Value>,
}

/// Commits `batch` to `document` through the same verified proposal as a publish.
fn build(document: &mut DocumentStore, batch: CommandBatch) -> Result<(), RuleProgramApplyError> {
    let proposal = document
        .prepare_proposal_with_context(batch, ProposalContext::rule_program())
        .map_err(|error| RuleProgramApplyError::Session(SessionError::Prepare(error)))?;
    document
        .commit_verified_proposal(&proposal)
        .map_err(|error| RuleProgramApplyError::Session(SessionError::Commit(error)))?;
    Ok(())
}

fn check(
    mut document: DocumentStore,
    source: &RuleProgramSource,
    container: &ContainerData,
    worker_path: Option<PathBuf>,
    cancelled: Arc<AtomicBool>,
) -> Result<Checked, RuleProgramApplyError> {
    let started = Instant::now();
    let plan = ketchup_application::plan_rule_program(&document, source)?;
    let mut report = plan.report();
    let before = document.current();
    let (change, after) = match plan.change {
        RuleProgramChange::Unchanged => ("unchanged", before.clone()),
        RuleProgramChange::SourceOnly => ("source_only", before.clone()),
        RuleProgramChange::Incremental(batch) => {
            build(&mut document, batch)?;
            ("incremental", document.current())
        }
        RuleProgramChange::Replacement => {
            let mut scratch = DocumentStore::new();
            let batch = ketchup_application::plan_rule_model_batch(
                &scratch,
                plan.evaluated.reference_model(),
            )?;
            build(&mut scratch, batch)?;
            ("created", scratch.current())
        }
    };
    let model = plan.evaluated.model;
    let exact = if ketchup_program::exact_candidates(&model).is_empty() {
        None
    } else {
        ketchup_application::verify_rule_program_exact(
            &after,
            &model,
            &mut report,
            container,
            worker_path,
            PROGRAM_EXACT_TIMEOUT.saturating_sub(started.elapsed()),
            cancelled,
        )
    };
    Ok(Checked {
        change,
        report,
        model,
        before,
        after,
        exact,
    })
}

impl Checked {
    fn result(&self, stamp: &Stamp, context: &CheckContext) -> Value {
        let mut result = program_edit_result(
            crate::program_edit::ProgramEdit::Unchanged,
            (&self.report, &self.model),
            &self.before,
            &self.after,
            stamp,
            context.undo_steps,
            self.exact.clone(),
        );
        result["change"] = json!(self.change);
        result["check_only"] = json!(true);
        result["canonical_mutation"] = json!(false);
        result["source_diff"] = context.source_diff.clone();
        result["requires_replace_document"] =
            json!(self.change == "created" && context.owns_content);
        result
    }
}

/// Where a queued request answers and how it is cancelled.
pub(super) struct QueuedReply {
    pub(super) id: u64,
    pub(super) reply: mpsc::SyncSender<Response>,
    pub(super) cancelled: Arc<AtomicBool>,
}

impl LiveBridge {
    /// The program a check request names: a whole source or a patch of the current one.
    fn checked_source(
        app: &KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<RuleProgramSource, &'static str> {
        let Request::CheckProgram {
            expected,
            source,
            edits,
            overrides,
            file_name,
        } = request
        else {
            return Err("invalid_params");
        };
        let request = match (source, edits) {
            (Some(source), None) => Request::ApplyProgram {
                expected,
                source,
                overrides,
                file_name,
                replace_document: false,
            },
            (None, Some(edits)) => Request::PatchProgram {
                expected: expected.ok_or_else(|| {
                    program_access::invalid(
                        "expected",
                        "A patch is checked against the stamp it was written for.",
                        "Pass expected from program read.",
                    )
                })?,
                edits,
            },
            _ => {
                return Err(program_access::invalid(
                    "source",
                    "Give either source (the whole program) or edits (a patch), not both.",
                    "Send the whole program as source, or old/new edits with expected.",
                ));
            }
        };
        Self::program_source(app, request, ui_busy, cancelled).map(|(source, _)| source)
    }

    fn check_context(app: &KetchupApp, source: &RuleProgramSource) -> CheckContext {
        CheckContext {
            undo_steps: app.undo_step_count(),
            owns_content: app.document.current().definitions().next().is_some(),
            source_diff: program_access::source_diff(
                app.document
                    .current_rule_program()
                    .map_or("", |program| program.source.as_str()),
                &source.source,
            ),
        }
    }

    /// The synchronous check used by the in-process harness.
    pub(super) fn check_program_now(
        app: &KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let source = Self::checked_source(app, request, ui_busy, cancelled)?;
        let context = Self::check_context(app, &source);
        let checked = check(
            app.document.fork_for_planning(),
            &source,
            &app.file.container_data,
            Self::worker_path(app),
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(program_failure)?;
        Ok(checked.result(&app.live_bridge_stamp(), &context))
    }

    pub(super) fn start_queued_check_program(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        queued: QueuedReply,
        request: Request,
        ui_busy: bool,
    ) {
        let QueuedReply {
            id,
            reply,
            cancelled,
        } = queued;
        take_error_details();
        if self.program_check_job.is_some() {
            Self::reply(
                app,
                id,
                &reply,
                Err(failure(
                    "busy",
                    "A program is already planning or checking geometry.",
                    json!({}),
                )),
            );
            return;
        }
        let source = match Self::checked_source(app, request, ui_busy, &cancelled) {
            Ok(source) => source,
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let check_context = Self::check_context(app, &source);
        let document = app.document.fork_for_planning();
        let container = app.file.container_data.clone();
        let worker_path = Self::worker_path(app);
        let worker_cancelled = Arc::clone(&cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-program-dry-run".into())
            .spawn(move || {
                let result = check(document, &source, &container, worker_path, worker_cancelled);
                let _ = sender.try_send(result);
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            Self::reply(
                app,
                id,
                &reply,
                Err(failed_because("job_worker_unavailable", error)),
            );
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::DryRun(DryRunJob {
            id,
            reply,
            cancelled,
            before: app.live_bridge_stamp(),
            started: Instant::now(),
            context: check_context,
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_program_dry_run(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        job: DryRunJob,
    ) {
        take_error_details();
        // The check answers for the document it started on.
        let result = if let Err(error) = Self::guard(app, &Some(job.before.clone())) {
            job.cancelled.store(true, Ordering::Release);
            Err(error)
        } else if let Err(error) = Self::require_request_authority(&job.cancelled) {
            Err(error)
        } else if job.started.elapsed() > PROGRAM_EXACT_TIMEOUT + Duration::from_secs(5) {
            job.cancelled.store(true, Ordering::Release);
            Err(failure(
                "job_worker_unavailable",
                "The program check timed out; no model changes were made.",
                json!({}),
            ))
        } else {
            match job.receiver.try_recv() {
                Ok(result) => result
                    .map(|checked| checked.result(&job.before, &job.context))
                    .map_err(program_failure),
                Err(mpsc::TryRecvError::Empty) => {
                    self.program_check_job = Some(ProgramCheckJob::DryRun(job));
                    context.request_repaint_after(Duration::from_millis(10));
                    return;
                }
                Err(error) => Err(failed_because("program_worker_disconnected", error)),
            }
        };
        Self::reply(app, job.id, &job.reply, result);
    }
}
