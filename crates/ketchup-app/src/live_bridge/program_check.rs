//! Program planning and exact checks run off the UI thread. Publication remains
//! one UI-thread transaction, guarded by the original document and request authority.
#[cfg(test)]
#[path = "program_check_failure_tests.rs"]
mod failure_tests;
#[cfg(test)]
#[path = "program_plan_tests.rs"]
mod tests;
use super::*;
use crate::program_edit::ProgramEdit;
use ketchup_application::{RuleProgramApplyError, RuleProgramPlan};
use ketchup_model::document::RuleProgramSource;
use ketchup_program::{ProgramModel, Report, exact_candidates};

/// Plans the program and lists its report, both off the UI thread when queued.
fn plan_with_report(
    document: &DocumentStore,
    source: &RuleProgramSource,
) -> Result<(RuleProgramPlan, Report), RuleProgramApplyError> {
    let plan = ketchup_application::plan_rule_program(document, source)?;
    let report = plan.report();
    Ok((plan, report))
}

pub(super) const PROGRAM_EXACT_TIMEOUT: Duration = Duration::from_secs(20);
const PROGRAM_EXACT_GRACE: Duration = Duration::from_secs(5);

pub(super) struct AppliedProgram {
    edit: ProgramEdit,
    report: Report,
    model: ProgramModel,
    before: Snapshot,
    after: Snapshot,
    stamp: Stamp,
    undo_steps: usize,
    source_diff: Value,
}

impl AppliedProgram {
    pub(super) fn result(&self, exact: Option<Value>) -> Value {
        let mut result = program_edit_result(
            self.edit,
            (&self.report, &self.model),
            &self.before,
            &self.after,
            &self.stamp,
            self.undo_steps,
            exact,
        );
        result["source_diff"] = self.source_diff.clone();
        result
    }

    fn needs_exact_check(&self) -> bool {
        !exact_candidates(&self.model).is_empty()
    }
}

pub(super) enum ProgramCheckJob {
    Planning(ProgramPlanJob),
    Checking(Box<ExactCheckJob>),
    Validating(super::program_validate::ProgramValidationJob),
    Measuring(super::measurement::MeasurementJob),
}

pub(super) struct ProgramPlanJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    source: RuleProgramSource,
    replace: bool,
    before: Stamp,
    receiver: mpsc::Receiver<Result<(RuleProgramPlan, Report), RuleProgramApplyError>>,
}

pub(super) struct ExactCheckJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    worker_cancelled: Arc<AtomicBool>,
    started: Instant,
    applied: AppliedProgram,
    receiver: mpsc::Receiver<(Report, Option<Value>)>,
}

fn unfinished(reason: &str) -> Value {
    json!({"state": "incomplete", "not_evaluated": [{"reason": reason}]})
}

impl LiveBridge {
    fn program_source(
        app: &KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<(RuleProgramSource, bool), &'static str> {
        let request = Self::expand_program_patch(app, request)?;
        let Request::ApplyProgram {
            expected,
            source,
            overrides,
            file_name,
            replace_document,
        } = request
        else {
            return Err("invalid_params");
        };
        Self::guard(app, &expected)?;
        Self::available(app, ui_busy)?;
        Self::require_request_authority(cancelled)?;
        let file_name = file_name
            .or_else(|| {
                app.document
                    .current_rule_program()
                    .map(|program| program.file_name.clone())
            })
            .unwrap_or_else(|| "model.star".to_owned());
        Ok((
            RuleProgramSource {
                file_name,
                source,
                overrides,
            },
            replace_document,
        ))
    }

    // The synchronous executor is used by the in-process harness. The live queue
    // uses start_queued_apply_program, but both publish through this same path.
    pub(super) fn apply_program(
        &mut self,
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<AppliedProgram, &'static str> {
        let (source, replace) = Self::program_source(app, request, ui_busy, cancelled)?;
        let before = app.live_bridge_stamp();
        let planned = plan_with_report(&app.document, &source).map_err(program_failure)?;
        self.publish_program(app, source, replace, planned, &before, cancelled)
    }

    fn publish_program(
        &mut self,
        app: &mut KetchupApp,
        source: RuleProgramSource,
        replace: bool,
        (plan, report): (RuleProgramPlan, Report),
        before_stamp: &Stamp,
        cancelled: &AtomicBool,
    ) -> Result<AppliedProgram, &'static str> {
        Self::guard(app, &Some(before_stamp.clone()))?;
        Self::require_request_authority(cancelled)?;
        let before = app.document.current();
        let source_diff = super::program_access::source_diff(
            app.document
                .current_rule_program()
                .map_or("", |program| program.source.as_str()),
            &source.source,
        );
        let (edit, evaluated) = app
            .publish_program_plan(source, replace, Vec::new(), plan)
            .map_err(program_failure)?;
        let model = evaluated.model;
        self.pending = None;
        let stamp = app.live_bridge_stamp();
        if stamp.document_id != before_stamp.document_id {
            self.invalidate_document_context();
        } else {
            self.query.invalidate();
        }
        self.observed = Some(stamp.clone());
        self.program_report = Some((stamp.clone(), report.clone(), "apply_report"));
        Ok(AppliedProgram {
            edit,
            report,
            model,
            before,
            after: app.document.current(),
            stamp,
            undo_steps: app.undo_step_count(),
            source_diff,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_queued_apply_program(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        request: Request,
        ui_busy: bool,
    ) {
        take_error_details();
        if self.program_check_job.is_some() {
            Self::reply(
                app,
                id,
                &reply,
                Err(failure(
                    "busy",
                    "An earlier program apply is still planning or checking its geometry.",
                    json!({}),
                )),
            );
            return;
        }
        let (source, replace) = match Self::program_source(app, request, ui_busy, &cancelled) {
            Ok(source) => source,
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let document = app.document.fork_for_planning();
        let before = app.live_bridge_stamp();
        let worker_source = source.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-live-program-plan".into())
            .spawn(move || {
                let result = plan_with_report(&document, &worker_source);
                let _ = sender.try_send(result);
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            Self::reply(
                app,
                id,
                &reply,
                Err(failure(
                    "job_worker_unavailable",
                    "Could not start the program planner.",
                    json!({"cause": error.to_string()}),
                )),
            );
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::Planning(ProgramPlanJob {
            id,
            reply,
            cancelled,
            source,
            replace,
            before,
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    fn poll_program_plan(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: ProgramPlanJob,
    ) {
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::Planning(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Self::reply(
                    app,
                    job.id,
                    &job.reply,
                    Err(failure(
                        "program_worker_disconnected",
                        "The program planner stopped before returning a result.",
                        json!({}),
                    )),
                );
                return;
            }
        };
        take_error_details();
        let result = Self::available(app, ui_busy(context)).and_then(|()| {
            let plan = result.map_err(program_failure)?;
            self.publish_program(
                app,
                job.source,
                job.replace,
                plan,
                &job.before,
                &job.cancelled,
            )
        });
        match result {
            Ok(applied) => {
                self.start_program_exact(app, context, job.id, job.reply, job.cancelled, applied)
            }
            Err(code) => Self::reply(app, job.id, &job.reply, Err(code)),
        }
    }

    fn start_program_exact(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        applied: AppliedProgram,
    ) {
        if !applied.needs_exact_check() {
            Self::reply(app, id, &reply, Ok(applied.result(None)));
            return;
        }
        let mut report = applied.report.clone();
        let model = applied.model.clone();
        let snapshot = applied.after.clone();
        let container = app.file.container_data.clone();
        let worker_path = Self::worker_path(app);
        let worker_cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&worker_cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-live-program-check".into())
            .spawn(move || {
                let exact = ketchup_application::verify_rule_program_exact(
                    &snapshot,
                    &model,
                    &mut report,
                    &container,
                    worker_path,
                    PROGRAM_EXACT_TIMEOUT,
                    thread_cancelled,
                );
                let _ = sender.try_send((report, exact));
                repaint.request_repaint();
            });
        if spawn.is_err() {
            Self::reply(
                app,
                id,
                &reply,
                Ok(applied.result(Some(unfinished("job_worker_unavailable")))),
            );
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::Checking(Box::new(ExactCheckJob {
            id,
            reply,
            cancelled,
            worker_cancelled,
            started: Instant::now(),
            applied,
            receiver,
        })));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_program_check_job(&mut self, app: &mut KetchupApp, context: &egui::Context) {
        let Some(job) = self.program_check_job.take() else {
            return;
        };
        let mut job = match job {
            ProgramCheckJob::Planning(job) => {
                self.poll_program_plan(app, context, job);
                return;
            }
            ProgramCheckJob::Checking(job) => job,
            ProgramCheckJob::Measuring(job) => {
                self.poll_measurement(app, context, job);
                return;
            }
            ProgramCheckJob::Validating(job) => {
                self.poll_program_validation(app, context, job);
                return;
            }
        };
        let exact = if job.cancelled.load(Ordering::Acquire) {
            job.worker_cancelled.store(true, Ordering::Release);
            Some(unfinished("request_cancelled"))
        } else if job.started.elapsed() > PROGRAM_EXACT_TIMEOUT + PROGRAM_EXACT_GRACE {
            job.worker_cancelled.store(true, Ordering::Release);
            Some(unfinished("exact_collision_timeout"))
        } else {
            match job.receiver.try_recv() {
                Ok((report, exact)) => {
                    job.applied.report = report;
                    Some(exact.unwrap_or_else(|| unfinished("exact_result_missing")))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(unfinished("exact_collision_worker_disconnected"))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            }
        };
        if let Some(exact) = exact {
            self.program_report = Some((
                job.applied.stamp.clone(),
                job.applied.report.clone(),
                "apply_report",
            ));
            Self::reply(app, job.id, &job.reply, Ok(job.applied.result(Some(exact))));
            context.request_repaint();
        } else {
            self.program_check_job = Some(ProgramCheckJob::Checking(job));
            context.request_repaint_after(Duration::from_millis(10));
        }
    }
}
