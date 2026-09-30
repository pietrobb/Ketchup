//! `apply_program` publishes on the UI thread at once. When parts whose solid
//! is not their box (profile bodies, moved faces, booleans) touch or overlap
//! others, the reply waits for the native exact pair check of the applied
//! solids, which runs on a worker thread so the window stays responsive.
use super::*;
use crate::program_edit::ProgramEdit;
use ketchup_program::{ProgramModel, Report, exact_candidates};

/// Host budget for the exact check of one applied program; the SDK waits longer.
pub(super) const PROGRAM_EXACT_TIMEOUT: Duration = Duration::from_secs(20);
/// Extra time for worker startup and graph preparation before the poller gives up.
const PROGRAM_EXACT_GRACE: Duration = Duration::from_secs(5);

/// A published program edit, before its reply is built.
pub(super) struct AppliedProgram {
    edit: ProgramEdit,
    report: Report,
    model: ProgramModel,
    before: Snapshot,
    after: Snapshot,
    stamp: Stamp,
    undo_steps: usize,
}

impl AppliedProgram {
    pub(super) fn result(&self, exact: Option<Value>) -> Value {
        program_edit_result(
            self.edit,
            &self.report,
            &self.before,
            &self.after,
            &self.stamp,
            self.undo_steps,
            exact,
        )
    }

    fn needs_exact_check(&self) -> bool {
        !exact_candidates(&self.model).is_empty()
    }
}

pub(super) struct ProgramCheckJob {
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
    /// Evaluates and publishes one `Request::ApplyProgram` as one Undo step.
    pub(super) fn apply_program(
        &mut self,
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<AppliedProgram, &'static str> {
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
        let file_name = file_name
            .or_else(|| {
                app.document
                    .current_rule_program()
                    .map(|program| program.file_name.clone())
            })
            .unwrap_or_else(|| "model.star".to_owned());
        Self::require_request_authority(cancelled)?;
        let before = app.document.current();
        let before_stamp = app.live_bridge_stamp();
        let (edit, report, model) = app
            .apply_program_source(
                ketchup_model::document::RuleProgramSource {
                    file_name,
                    source,
                    overrides,
                },
                replace_document,
            )
            .map_err(program_failure)?;
        self.pending = None;
        let stamp = app.live_bridge_stamp();
        if stamp.document_id != before_stamp.document_id {
            self.invalidate_document_context();
        } else {
            self.query.invalidate();
        }
        self.observed = Some(stamp.clone());
        Ok(AppliedProgram {
            edit,
            report,
            model,
            before,
            after: app.document.current(),
            stamp,
            undo_steps: app.undo_step_count(),
        })
    }

    /// Publishes the program, then replies at once or after the exact check.
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
            Self::reply(app, id, &reply, Err("busy"));
            return;
        }
        let applied = match self.apply_program(app, request, ui_busy, &cancelled) {
            Ok(applied) => applied,
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
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
            let result = applied.result(Some(unfinished("job_worker_unavailable")));
            Self::reply(app, id, &reply, Ok(result));
            return;
        }
        self.program_check_job = Some(ProgramCheckJob {
            id,
            reply,
            cancelled,
            worker_cancelled,
            started: Instant::now(),
            applied,
            receiver,
        });
        context.request_repaint_after(Duration::from_millis(10));
    }

    /// Replies to a finished exact check. The edit is already published, so a
    /// cancelled or overdue check still answers, with the overlaps unverified.
    pub(super) fn poll_program_check_job(&mut self, app: &KetchupApp, context: &egui::Context) {
        let Some(mut job) = self.program_check_job.take() else {
            return;
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
                    Some(exact.unwrap_or_else(|| json!({"state": "verified"})))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(unfinished("exact_collision_worker_disconnected"))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            }
        };
        if let Some(exact) = exact {
            Self::reply(app, job.id, &job.reply, Ok(job.applied.result(Some(exact))));
            context.request_repaint();
        } else {
            self.program_check_job = Some(job);
            context.request_repaint_after(Duration::from_millis(10));
        }
    }
}
